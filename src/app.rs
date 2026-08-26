use uuid::Uuid;

use crate::{
    action::{Action, Command},
    cli::ConfirmationPolicy,
    editor::{EditorEffect, EditorError, EditorWorkspace},
    model::{
        editor::{EditorMode, EditorRenderSnapshot, EditorViewport},
        profile_manager::{
            ProfileField, ProfileManagerPage, ProfileManagerState, ProfileOperation,
        },
        tab::{
            CompletionPopup, ConsoleTab, ExecutionResult, LastExecution, OutputEntry, OutputKind,
            ResultView,
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
    profile::{ConnectionProfile, DatabaseKind},
    sql::{self, CompletionScheduleKey, ScopeSource, SqlDialect},
};

pub struct App {
    pub profiles: Vec<ConnectionProfile>,
    pub connection: ConnectionState,
    pub explorer: ExplorerState,
    pub tabs: Vec<ConsoleTab>,
    pub active_tab: usize,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    pub profile_manager: Option<ProfileManagerState>,
    pub should_quit: bool,
    next_console_number: usize,
    connection_request_generation: u64,
    connection_terminal_generation: u64,
    editor: EditorWorkspace,
    confirmation_policy: ConfirmationPolicy,
    deferred: DeferredIntentQueue,
    resolving_deferred: Option<DeferredTransactionPrompt>,
}

impl App {
    pub fn new(profiles: Vec<ConnectionProfile>) -> Self {
        Self::with_confirmation_policy(profiles, ConfirmationPolicy::RiskyOnly)
    }

    pub fn with_confirmation_policy(
        profiles: Vec<ConnectionProfile>,
        confirmation_policy: ConfirmationPolicy,
    ) -> Self {
        let tab = ConsoleTab::new("console");
        let tab_id = tab.id;
        let mut editor = EditorWorkspace::new();
        editor.open_console(tab_id, "");
        let mut app = Self {
            profiles,
            connection: ConnectionState::default(),
            explorer: ExplorerState::default(),
            tabs: vec![tab],
            active_tab: 0,
            focus: Focus::Editor,
            overlay: None,
            profile_manager: None,
            should_quit: false,
            next_console_number: 2,
            connection_request_generation: 0,
            connection_terminal_generation: 0,
            editor,
            confirmation_policy,
            deferred: DeferredIntentQueue::default(),
            resolving_deferred: None,
        };
        app.assign_default_target(tab_id);
        app
    }

    pub fn set_confirmation_policy(&mut self, policy: ConfirmationPolicy) {
        self.confirmation_policy = policy;
    }

    pub fn active_console(&self) -> &ConsoleTab {
        &self.tabs[self.active_tab]
    }

    pub fn active_console_mut(&mut self) -> &mut ConsoleTab {
        &mut self.tabs[self.active_tab]
    }

    pub fn active_editor_text(&self) -> Result<String, EditorError> {
        self.editor_text(self.active_console().id)
    }

    pub fn editor_text(&self, tab_id: Uuid) -> Result<String, EditorError> {
        self.editor.text(tab_id)
    }

    pub fn active_editor_revision(&self) -> u64 {
        self.editor
            .revision(self.active_console().id)
            .unwrap_or_default()
    }

    pub fn active_editor_mode(&self) -> EditorMode {
        self.editor
            .mode(self.active_console().id)
            .unwrap_or(EditorMode::Normal)
    }

    pub fn active_editor_viewport(&self) -> Result<EditorViewport, EditorError> {
        self.editor.viewport(self.active_console().id)
    }

    pub fn active_editor_render_snapshot(
        &self,
        viewport: EditorViewport,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        self.editor.render_snapshot_with_dialect(
            self.active_console().id,
            viewport,
            self.sql_dialect(),
        )
    }

    pub fn active_profile(&self) -> Option<&ConnectionProfile> {
        let profile_id = self.connection.profile_id?;
        self.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
    }

    fn assign_default_target(&mut self, tab_id: Uuid) {
        let target = self
            .profiles
            .first()
            .map(crate::model::execution_target::ExecutionTarget::from_profile);
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.execution_target = target;
        }
    }

    pub fn update(&mut self, action: Action) -> Vec<Command> {
        match action {
            Action::NewConsole => {
                let name = format!("console_{}", self.next_console_number);
                self.next_console_number += 1;
                let tab = ConsoleTab::new(name);
                let id = tab.id;
                self.tabs.push(tab);
                self.editor.open_console(id, "");
                self.assign_default_target(id);
                self.active_tab = self.tabs.len() - 1;
                self.focus = Focus::Editor;
                vec![Command::PersistWorkspace]
            }
            Action::CloseActiveTab => {
                if self.tabs.len() > 1 {
                    let id = self.active_console().id;
                    if self.transaction_needs_exit(id) {
                        return self.defer_intent(DeferredIntent::CloseConsole, [id]);
                    }
                    self.tabs.remove(self.active_tab);
                    self.editor.close_console(id);
                    self.active_tab = self.active_tab.saturating_sub(1);
                    vec![Command::PersistWorkspace]
                } else {
                    Vec::new()
                }
            }
            Action::NextTab => {
                self.active_tab = (self.active_tab + 1) % self.tabs.len();
                Vec::new()
            }
            Action::PreviousTab => {
                self.active_tab = self
                    .active_tab
                    .checked_sub(1)
                    .unwrap_or(self.tabs.len() - 1);
                Vec::new()
            }
            Action::ActivateTab(index) => {
                if index < self.tabs.len() {
                    self.active_tab = index;
                }
                Vec::new()
            }
            Action::FocusNext => {
                self.focus = self.focus.next();
                Vec::new()
            }
            Action::FocusPrevious => {
                self.focus = self.focus.previous();
                Vec::new()
            }
            Action::Focus(focus) => {
                self.focus = focus;
                Vec::new()
            }
            Action::ShowHelp => {
                self.overlay = Some(Overlay::Help(self.focus));
                Vec::new()
            }
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
                if self.profiles.is_empty() {
                    manager.start_new(DatabaseKind::Postgres);
                }
                self.profile_manager = Some(manager);
                self.overlay = Some(Overlay::ProfileManager);
                Vec::new()
            }
            Action::CloseProfileManager => {
                self.close_profile_manager();
                Vec::new()
            }
            Action::ProfileMove(delta) => {
                let profile_count = self.profiles.len();
                if let Some(manager) = self.idle_profile_manager_mut(ProfileManagerPage::List) {
                    manager.move_selection(delta, profile_count);
                    manager.message = None;
                }
                Vec::new()
            }
            Action::ProfileStartNew => {
                if let Some(manager) = self.idle_profile_manager_mut(ProfileManagerPage::List) {
                    manager.start_new(DatabaseKind::Postgres);
                }
                Vec::new()
            }
            Action::ProfileStartEdit => {
                let profile = self.selected_profile().cloned();
                if let (Some(profile), Some(manager)) = (
                    profile,
                    self.idle_profile_manager_mut(ProfileManagerPage::List),
                ) {
                    let has_stored_credential = profile.secret_ref.is_some();
                    manager.start_edit(&profile, has_stored_credential);
                }
                Vec::new()
            }
            Action::ProfileRequestDelete => {
                self.request_profile_delete();
                Vec::new()
            }
            Action::ProfileConfirmDelete => self.confirm_profile_delete(),
            Action::ProfileCancelDelete => {
                if let Some(manager) =
                    self.idle_profile_manager_mut(ProfileManagerPage::ConfirmDelete)
                {
                    manager.page = ProfileManagerPage::List;
                    manager.message = None;
                }
                Vec::new()
            }
            Action::ProfileConnectSelected => self.connect_selected_profile(),
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
            Action::ProfileCycle(delta) => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.cycle(delta);
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
            Action::ProfileTest => self.test_profile_draft(),
            Action::ProfileSave { connect } => self.save_profile_draft(connect),
            Action::ProfileTestSucceeded { request_id, server } => {
                if let Some(manager) =
                    self.matching_profile_operation(request_id, &[ProfileOperation::Testing])
                {
                    manager.operation = None;
                    manager.message = Some(format!(
                        "Connection succeeded: {} ({})",
                        server.version, server.database
                    ));
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
            Action::ProfileSaved {
                request_id,
                profile,
                warning,
                connect,
            } => self.profile_saved(request_id, profile, warning, connect),
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
                self.connection.status = if self.connection.profile_id.is_some() {
                    ConnectionStatus::Connected
                } else {
                    ConnectionStatus::Failed
                };
                self.connection.error = Some(message.clone());
                if let Some(manager) = self.profile_manager.as_mut()
                    && manager
                        .operation
                        .is_some_and(|operation| operation != ProfileOperation::Connecting)
                {
                    manager.message = Some(message);
                    return Vec::new();
                }
                let has_stored_credential = profile.secret_ref.is_some();
                let mut manager = ProfileManagerState::default();
                manager.start_edit(&profile, has_stored_credential);
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
                }
                if active_matches {
                    self.connection.profile_id = None;
                    self.connection.generation = 0;
                    self.connection.server = None;
                    self.connection.error = None;
                    self.explorer = ExplorerState::default();
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
                let id = self.active_console().id;
                self.active_console_mut().completion = None;
                if self.editor.key(id, key).is_err() {
                    return Vec::new();
                }
                self.apply_editor_effects()
            }
            Action::EditorPaste(text) => {
                let id = self.active_console().id;
                self.active_console_mut().completion = None;
                if self.editor.paste(id, &text).is_err() {
                    return Vec::new();
                }
                self.apply_editor_effects()
            }
            Action::EditorViewportChanged(viewport) => {
                let id = self.active_console().id;
                let _ = self.editor.set_viewport(id, viewport);
                Vec::new()
            }
            Action::EditorScroll { rows, columns } => {
                let id = self.active_console().id;
                let _ = self.editor.scroll(id, rows, columns);
                Vec::new()
            }
            Action::ReplaceEditor(text) => {
                let id = self.active_console().id;
                let _ = self.editor.set_text(id, &text);
                vec![Command::PersistWorkspace]
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
                    && matches!(
                        tab.transaction_state,
                        TransactionState::Starting | TransactionState::Active
                    )
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
                let choice = match self.overlay.as_ref() {
                    Some(Overlay::TransactionExitConfirm { choice, .. }) => *choice,
                    _ => TransactionExitChoice::Cancel,
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
            Action::ConfirmManualCancellation => {
                let Some(Overlay::ManualCancelConfirm { intent, focus }) = self.overlay.take()
                else {
                    return Vec::new();
                };
                if focus != ManualCancelFocus::CancelQueryAndRollback {
                    return Vec::new();
                }
                let current = self.tabs.iter().find(|tab| tab.id == intent.console_id);
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
                    .find(|tab| tab.id == intent.console_id)
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
                let Some(connection) = self.database_command_identity() else {
                    return Vec::new();
                };
                vec![Command::LoadCatalog {
                    profile_id: connection.profile_id,
                    generation: connection.generation,
                }]
            }
            Action::PreviewSelected => self.preview_selected(),
            Action::DdlSelected => self.ddl_selected(),
            Action::RequestConnect(profile_id) => self.request_connection(profile_id),
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
                self.connection_terminal_generation = generation;
                self.connection.profile_id = Some(profile_id);
                self.connection.generation = generation;
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
                self.explorer.connection_changed();
                for tab in &mut self.tabs {
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
                vec![Command::LoadCatalog {
                    profile_id,
                    generation,
                }]
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
                    self.connection.status = if self.connection.profile_id.is_some() {
                        ConnectionStatus::Connected
                    } else {
                        ConnectionStatus::Failed
                    };
                    self.connection.error = Some(message.clone());
                    if let Some(manager) = self.profile_manager.as_mut()
                        && manager.operation == Some(ProfileOperation::Connecting)
                    {
                        manager.operation = None;
                        manager.message = Some(message);
                    }
                }
                Vec::new()
            }
            Action::CatalogLoaded {
                profile_id,
                generation,
                nodes,
            } => {
                if self.active_connection_matches(profile_id, generation) {
                    self.explorer.set_nodes(nodes);
                }
                Vec::new()
            }
            Action::CatalogFailed {
                profile_id,
                generation,
                message,
            } => {
                if self.active_connection_matches(profile_id, generation) {
                    self.connection.error = Some(message);
                }
                Vec::new()
            }
            Action::QueryFinished {
                tab_id,
                generation,
                connection,
                outcome,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
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
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
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
                    let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id).unwrap();
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
                    let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id).unwrap();
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
                    let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id).unwrap();
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
                    && let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
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
                    let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id).unwrap();
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
                    let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id).unwrap();
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
                    let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id).unwrap();
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
                    let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id).unwrap();
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
                    let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id).unwrap();
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
            Action::PreviewFinished {
                tab_id,
                generation,
                sql,
                outcome,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return Vec::new();
                };
                if tab.generation != generation {
                    return Vec::new();
                }
                let _ = self.editor.set_text(tab_id, &sql);
                let _ = self.editor.set_mode(tab_id, EditorMode::Normal);
                let rows = outcome.stats.row_count;
                let total_ms = outcome.stats.total().as_millis();
                tab.outcome = Some(outcome);
                tab.query_status = QueryStatus::Idle;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Success,
                    message: format!("{rows} row(s) previewed in {total_ms} ms"),
                });
                tab.result_view = ResultView::Data;
                Vec::new()
            }
            Action::DdlLoaded {
                tab_id,
                generation,
                ddl,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return Vec::new();
                };
                if tab.generation != generation {
                    return Vec::new();
                }
                let _ = self.editor.set_text(tab_id, &ddl);
                let _ = self.editor.set_mode(tab_id, EditorMode::Normal);
                tab.query_status = QueryStatus::Idle;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Success,
                    message: "Object DDL loaded".to_owned(),
                });
                self.focus = Focus::Editor;
                Vec::new()
            }
            Action::ExplorerMove(delta) => {
                self.explorer.move_selection(delta);
                Vec::new()
            }
            Action::ExplorerSelect(index) => {
                if index < self.explorer.visible().len() {
                    self.explorer.selected = index;
                }
                Vec::new()
            }
            Action::GridMove { rows, columns } => {
                let tab = self.active_console_mut();
                let (row_count, column_count) = tab
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.result_sets.last())
                    .map(|result| (result.rows.len(), result.columns.len()))
                    .unwrap_or((0, 0));
                tab.selected_row = move_bounded(tab.selected_row, rows, row_count);
                tab.selected_column = move_bounded(tab.selected_column, columns, column_count);
                Vec::new()
            }
            Action::GridSelect { row, column } => {
                let tab = self.active_console_mut();
                let (row_count, column_count) = tab
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.result_sets.last())
                    .map(|result| (result.rows.len(), result.columns.len()))
                    .unwrap_or((0, 0));
                tab.selected_row = row.min(row_count.saturating_sub(1));
                tab.selected_column = column.min(column_count.saturating_sub(1));
                Vec::new()
            }
            Action::ExplorerToggle => {
                self.explorer.toggle_selected();
                Vec::new()
            }
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
                    .filter(|tab| self.transaction_needs_exit(tab.id))
                    .map(|tab| tab.id)
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
        let Some(manager) = self.profile_manager.as_mut() else {
            return;
        };
        if manager.operation.is_some() {
            return;
        }
        if manager.page != ProfileManagerPage::List && !self.profiles.is_empty() {
            manager.page = ProfileManagerPage::List;
            manager.draft = None;
            manager.selected_field = ProfileField::Kind;
            manager.message = None;
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
            .find(|tab| tab.id == console_id)
            .is_some_and(|tab| tab.transaction_state != TransactionState::Idle)
    }

    fn defer_intent<I>(&mut self, intent: DeferredIntent, console_ids: I) -> Vec<Command>
    where
        I: IntoIterator<Item = Uuid>,
    {
        for console_id in console_ids {
            let Some(tab) = self.tabs.iter().find(|tab| tab.id == console_id) else {
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
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == prompt.console_id) else {
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
            .find(|tab| tab.id == prompt.console_id)
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
                    if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                        self.tabs.remove(index);
                        self.editor.close_console(id);
                        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
                    }
                }
                vec![Command::PersistWorkspace]
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
                .find(|tab| tab.id == console_id)
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
            .find(|tab| tab.id == console_id)
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

    fn selected_profile(&self) -> Option<&ConnectionProfile> {
        let selected = self.profile_manager.as_ref()?.selected;
        self.profiles.get(selected)
    }

    fn request_profile_delete(&mut self) {
        let Some(profile_id) = self.selected_profile().map(|profile| profile.id) else {
            return;
        };
        let blocked = self.connection.profile_id == Some(profile_id) && self.has_running_query();
        let Some(manager) = self.idle_profile_manager_mut(ProfileManagerPage::List) else {
            return;
        };
        if blocked {
            manager.message = Some("Cancel the running query before deleting this profile".into());
        } else {
            manager.page = ProfileManagerPage::ConfirmDelete;
            manager.message = None;
        }
    }

    fn confirm_profile_delete(&mut self) -> Vec<Command> {
        let Some(profile_id) = self.selected_profile().map(|profile| profile.id) else {
            return Vec::new();
        };
        let blocked = self.connection.profile_id == Some(profile_id) && self.has_running_query();
        let active_console_id = self.active_console().id;
        let should_defer = self.connection.profile_id == Some(profile_id)
            && self.transaction_needs_exit(active_console_id);
        let deferred_console_ids = self
            .tabs
            .iter()
            .filter(|tab| self.transaction_needs_exit(tab.id))
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        let Some(manager) = self.idle_profile_manager_mut(ProfileManagerPage::ConfirmDelete) else {
            return Vec::new();
        };
        if blocked {
            manager.page = ProfileManagerPage::List;
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

    fn connect_selected_profile(&mut self) -> Vec<Command> {
        let Some(profile_id) = self.selected_profile().map(|profile| profile.id) else {
            return Vec::new();
        };
        if self.connection.profile_id == Some(profile_id)
            && self.connection.status == ConnectionStatus::Connected
            && self.connection.pending_profile_id.is_none()
        {
            if let Some(manager) = self.idle_profile_manager_mut(ProfileManagerPage::List) {
                manager.message = Some("Profile is already connected".into());
            }
            return Vec::new();
        }
        if self.connection.profile_id != Some(profile_id) && self.has_running_query() {
            if let Some(manager) = self.idle_profile_manager_mut(ProfileManagerPage::List) {
                manager.message = Some("Cancel the running query before switching profiles".into());
            }
            return Vec::new();
        }
        if self.connection.profile_id != Some(profile_id) {
            let ids = self
                .tabs
                .iter()
                .filter(|tab| self.transaction_needs_exit(tab.id))
                .map(|tab| tab.id)
                .collect::<Vec<_>>();
            if !ids.is_empty() {
                return self.defer_intent(
                    DeferredIntent::SwitchConnection {
                        profile_id,
                        generation: 0,
                    },
                    ids,
                );
            }
        }
        if self
            .idle_profile_manager_mut(ProfileManagerPage::List)
            .is_none()
        {
            return Vec::new();
        }
        let commands = self.request_connection(profile_id);
        if !commands.is_empty()
            && let Some(manager) = self.profile_manager.as_mut()
        {
            manager.operation = Some(ProfileOperation::Connecting);
            manager.message = Some("Connecting...".into());
        }
        commands
    }

    fn test_profile_draft(&mut self) -> Vec<Command> {
        let profiles = &self.profiles;
        let Some(manager) = self.profile_manager.as_mut().filter(|manager| {
            manager.page == ProfileManagerPage::Form && manager.operation.is_none()
        }) else {
            return Vec::new();
        };
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
        manager.operation = Some(ProfileOperation::Testing);
        manager.message = Some("Testing connection...".into());
        vec![Command::TestProfile {
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
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|existing| existing.id == profile_id)
        {
            *existing = profile;
        } else {
            self.profiles.push(profile);
        }
        let selected = self
            .profiles
            .iter()
            .position(|profile| profile.id == profile_id)
            .unwrap_or(0);
        if let Some(manager) = self.profile_manager.as_mut() {
            manager.page = ProfileManagerPage::List;
            manager.selected = selected;
            manager.draft = None;
            manager.selected_field = ProfileField::Kind;
            manager.operation = None;
            manager.message = warning.or_else(|| Some("Profile saved".into()));
        }

        if !connect {
            if self.connection.profile_id == Some(profile_id) && self.has_running_query() {
                if let Some(manager) = self.profile_manager.as_mut() {
                    manager.message =
                        Some("Profile saved; cancel the running query before reconnecting".into());
                }
                return Vec::new();
            }
            return self.retire_profile_connections(profile_id, None);
        }
        if self.has_running_query() {
            if let Some(manager) = self.profile_manager.as_mut() {
                manager.message =
                    Some("Profile saved; cancel the running query before connecting".into());
            }
            return Vec::new();
        }
        let commands = self.request_connection(profile_id);
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
        self.profiles.retain(|profile| profile.id != profile_id);
        if let Some(manager) = self.profile_manager.as_mut() {
            manager.page = ProfileManagerPage::List;
            manager.selected = manager.selected.min(self.profiles.len().saturating_sub(1));
            manager.draft = None;
            manager.operation = None;
            manager.message = Some("Profile deleted".into());
        }
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
        self.connection.status = ConnectionStatus::Connecting;
        self.connection.error = None;
        vec![Command::Connect {
            profile_id,
            generation,
        }]
    }

    fn retire_profile_connections(
        &mut self,
        profile_id: Uuid,
        runtime_active: Option<ConnectionIdentity>,
    ) -> Vec<Command> {
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
            self.explorer = ExplorerState::default();
        }
        if let Some(connection) = pending {
            identities.push(connection);
            self.connection.pending_profile_id = None;
            self.connection.pending_generation = None;
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
        identities
            .into_iter()
            .map(|connection| Command::Disconnect { connection })
            .collect()
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
            Some(ProfileOperation::Deleting) => self
                .profiles
                .get(manager.selected)
                .is_some_and(|profile| Some(profile.id) == self.connection.profile_id),
            _ => false,
        }
    }

    fn has_running_query(&self) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.query_status == QueryStatus::Running)
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
                        self.complete_now();
                    } else if let Some(key) = self.completion_key() {
                        commands.push(Command::ScheduleCompletion(key));
                    }
                    continue;
                }
                EditorEffect::Message(message) => {
                    self.status_message(&message);
                    continue;
                }
                EditorEffect::BackwardSearch => continue,
                EditorEffect::ToggleTransaction => Action::SetTransactionMode(
                    if self.active_console().transaction_mode == TransactionMode::Manual {
                        TransactionMode::Auto
                    } else {
                        TransactionMode::Manual
                    },
                ),
                EditorEffect::ClearTransactionOutcome => Action::ClearTransactionOutcome,
                EditorEffect::SetConnectionTarget(name) => {
                    commands.extend(self.set_connection_target(&name));
                    continue;
                }
                EditorEffect::SetDatabaseTarget(database) => {
                    commands.extend(self.set_database_target(&database));
                    continue;
                }
                EditorEffect::SetSchemaTarget(schema) => {
                    commands.extend(self.set_schema_target(&schema));
                    continue;
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
        Some(CompletionScheduleKey {
            console_id: self.active_console().id,
            document_revision: self.active_editor_revision(),
            connection: self.connection.active_identity()?,
            catalog_generation: self.explorer.catalog_generation,
        })
    }

    fn set_connection_target(&mut self, name: &str) -> Vec<Command> {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .cloned()
        else {
            self.status_message(&format!("Connection profile not found: {name}"));
            return Vec::new();
        };
        if self.transaction_needs_exit(self.active_console().id) {
            self.status_message(
                "Rollback or commit the active transaction before changing connection",
            );
            return Vec::new();
        }
        let target = crate::model::execution_target::ExecutionTarget::from_profile(&profile);
        self.active_console_mut().execution_target = Some(target);
        self.request_connection(profile.id)
    }

    fn set_database_target(&mut self, database: &str) -> Vec<Command> {
        if database.is_empty() {
            self.status_message("Database target cannot be empty");
            return Vec::new();
        }
        if self.transaction_needs_exit(self.active_console().id) {
            self.status_message(
                "Rollback or commit the active transaction before changing database",
            );
            return Vec::new();
        }
        let target = self
            .active_console_mut()
            .execution_target
            .get_or_insert_with(|| crate::model::execution_target::ExecutionTarget {
                profile_id: Uuid::nil(),
                database: String::new(),
                schema: None,
            });
        target.database = database.to_owned();
        target.schema = None;
        self.status_message("Execution database target changed; reconnect before executing");
        Vec::new()
    }

    fn set_schema_target(&mut self, schema: &str) -> Vec<Command> {
        if schema.is_empty() {
            self.status_message("Schema target cannot be empty");
            return Vec::new();
        }
        if self.transaction_needs_exit(self.active_console().id) {
            self.status_message("Rollback or commit the active transaction before changing schema");
            return Vec::new();
        }
        let target = self
            .active_console_mut()
            .execution_target
            .get_or_insert_with(|| crate::model::execution_target::ExecutionTarget {
                profile_id: Uuid::nil(),
                database: String::new(),
                schema: None,
            });
        target.schema = Some(schema.to_owned());
        self.status_message("Execution schema target changed; reconnect before executing");
        Vec::new()
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
        let candidates = sql::complete(
            &text,
            cursor,
            self.sql_dialect(),
            &self.explorer.completion_index,
            None,
        );
        self.active_console_mut().completion =
            (!candidates.is_empty()).then_some(CompletionPopup {
                candidates,
                selected: 0,
            });
        Vec::new()
    }

    fn accept_completion(&mut self) -> Vec<Command> {
        let id = self.active_console().id;
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
        let id = self.active_console().id;
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
        let tab = self.active_console_mut();
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
        if self.active_console().query_status == QueryStatus::Running {
            self.status_message("A query is already running in this console");
            return Vec::new();
        }
        match sql::classify_transaction_sql(&scope.sql, dialect) {
            sql::TransactionSqlClassification::Control(control) => {
                return self.dispatch_transaction_sql(tab_id, connection, control, scope.sql);
            }
            sql::TransactionSqlClassification::Unsupported(_) => {}
            sql::TransactionSqlClassification::Data { .. } => {}
        }
        let tab = self.active_console();
        let draft = sql::ExecutionDraft::new(
            tab_id,
            tab.generation,
            connection,
            tab.execution_target.clone().unwrap_or_else(|| {
                crate::model::execution_target::ExecutionTarget {
                    profile_id: connection.profile_id,
                    database: String::new(),
                    schema: None,
                }
            }),
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
        sql: String,
    ) -> Vec<Command> {
        use sql::TransactionControl;
        let tab = self.active_console();
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
                self.dispatch_manual_sql(tab_id, connection, sql)
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
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == draft.console_id) else {
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
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == draft.target.profile_id)
        else {
            return Err("Execution target profile no longer exists".to_owned());
        };
        if draft.target.profile_id != draft.connection.profile_id {
            return Err("Execution target profile does not match active connection".to_owned());
        }
        if !draft.target.is_valid(profile) {
            return Err("Execution target database or schema is invalid".to_owned());
        }
        if tab.execution_target.as_ref() != Some(&draft.target) {
            return Err("Execution draft is stale: execution target changed".to_owned());
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
        let tab = self.tabs.iter_mut().find(|tab| tab.id == draft.console_id);
        let Some(tab) = tab else {
            return Vec::new();
        };
        if draft.transaction_mode == TransactionMode::Manual {
            let connection = draft.connection;
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
            tab_id: draft.console_id,
            generation,
            sql: draft.sql,
        }]
    }

    fn retain_execution(&mut self, draft: sql::ExecutionDraft, result: ExecutionResult) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == draft.console_id) {
            tab.last_execution = Some(LastExecution { draft, result });
        }
    }

    fn active_editor_revision_for(&self, id: Uuid) -> u64 {
        self.editor.revision(id).unwrap_or_default()
    }

    fn status_message(&mut self, message: &str) {
        self.connection.error = Some(message.to_owned());
    }

    fn preview_selected(&mut self) -> Vec<Command> {
        let Some(connection) = self.database_command_identity() else {
            return Vec::new();
        };
        let Some(node) = self.explorer.selected_node().cloned() else {
            return Vec::new();
        };
        if !matches!(
            node.kind,
            crate::db::catalog::CatalogKind::Table | crate::db::catalog::CatalogKind::View
        ) {
            self.explorer.toggle_selected();
            return Vec::new();
        }
        let Some(schema) = node
            .id
            .native_path
            .get(node.id.native_path.len().saturating_sub(2))
            .cloned()
        else {
            return Vec::new();
        };
        let mut tab = ConsoleTab::new(format!("{} data", node.name));
        tab.generation = 1;
        tab.query_status = QueryStatus::Running;
        let tab_id = tab.id;
        let generation = tab.generation;
        self.tabs.push(tab);
        self.editor.open_console(
            tab_id,
            &format!("-- Loading preview for {schema}.{}", node.name),
        );
        let _ = self.editor.set_mode(tab_id, EditorMode::Normal);
        self.active_tab = self.tabs.len() - 1;
        self.focus = Focus::Results;
        vec![Command::PreviewTable {
            connection,
            tab_id,
            generation,
            schema,
            name: node.name,
        }]
    }

    fn ddl_selected(&mut self) -> Vec<Command> {
        let Some(connection) = self.database_command_identity() else {
            return Vec::new();
        };
        let Some(node) = self.explorer.selected_node().cloned() else {
            return Vec::new();
        };
        if !matches!(
            node.kind,
            crate::db::catalog::CatalogKind::Table
                | crate::db::catalog::CatalogKind::View
                | crate::db::catalog::CatalogKind::Index
                | crate::db::catalog::CatalogKind::Trigger
        ) {
            return Vec::new();
        }
        let Some(schema) = node
            .id
            .native_path
            .get(node.id.native_path.len().saturating_sub(2))
            .cloned()
        else {
            return Vec::new();
        };
        let mut tab = ConsoleTab::new(format!("{} DDL", node.name));
        tab.generation = 1;
        tab.query_status = QueryStatus::Running;
        let tab_id = tab.id;
        let generation = tab.generation;
        self.tabs.push(tab);
        self.editor.open_console(tab_id, "");
        self.active_tab = self.tabs.len() - 1;
        self.focus = Focus::Editor;
        vec![Command::LoadDdl {
            connection,
            tab_id,
            generation,
            kind: node.kind,
            schema,
            name: node.name,
        }]
    }

    fn active_connection_matches(&self, profile_id: Uuid, generation: u64) -> bool {
        self.connection.profile_id == Some(profile_id) && self.connection.generation == generation
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
                .find(|tab| tab.id == tab_id)
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
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        action::{Action, Command},
        db::query::{QueryOutcome, QueryStats, ResultSet},
        model::workspace::{ConnectionStatus, Focus, Overlay, QueryStatus},
        profile::import_connection_url,
    };
    use uuid::Uuid;

    use super::App;

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
                .map(|tab| tab.name.as_str())
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
        assert_eq!(app.overlay, Some(Overlay::Help(Focus::Explorer)));
        app.update(Action::DismissOverlay);
        assert_eq!(app.overlay, None);
    }

    #[test]
    fn stale_query_results_cannot_replace_newer_runs() {
        let mut app = App::new(Vec::new());
        app.connection.profile_id = Some(Uuid::new_v4());
        app.connection.generation = 1;
        app.connection.status = ConnectionStatus::Connected;
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
        let commands = app.update(Action::RequestConnect(profile_id));
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
}
