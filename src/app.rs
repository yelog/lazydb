use uuid::Uuid;

use crate::{
    action::{Action, Command},
    model::{
        editor::EditorMode,
        profile_manager::{
            ProfileField, ProfileManagerPage, ProfileManagerState, ProfileOperation,
        },
        tab::{ConsoleTab, OutputEntry, OutputKind, ResultView},
        workspace::{
            ConnectionState, ConnectionStatus, ExplorerState, Focus, Overlay, QueryStatus,
        },
    },
    profile::{ConnectionProfile, DatabaseKind},
};

#[derive(Clone, Debug)]
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
}

impl App {
    pub fn new(profiles: Vec<ConnectionProfile>) -> Self {
        Self {
            profiles,
            connection: ConnectionState::default(),
            explorer: ExplorerState::default(),
            tabs: vec![ConsoleTab::new("console")],
            active_tab: 0,
            focus: Focus::Editor,
            overlay: None,
            profile_manager: None,
            should_quit: false,
            next_console_number: 2,
        }
    }

    pub fn active_console(&self) -> &ConsoleTab {
        &self.tabs[self.active_tab]
    }

    pub fn active_console_mut(&mut self) -> &mut ConsoleTab {
        &mut self.tabs[self.active_tab]
    }

    pub fn active_profile(&self) -> Option<&ConnectionProfile> {
        let profile_id = self.connection.profile_id?;
        self.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
    }

    pub fn update(&mut self, action: Action) -> Vec<Command> {
        match action {
            Action::NewConsole => {
                let name = format!("console_{}", self.next_console_number);
                self.next_console_number += 1;
                self.tabs.push(ConsoleTab::new(name));
                self.active_tab = self.tabs.len() - 1;
                self.focus = Focus::Editor;
                vec![Command::PersistWorkspace]
            }
            Action::CloseActiveTab => {
                if self.tabs.len() > 1 {
                    self.tabs.remove(self.active_tab);
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
                if self.overlay == Some(Overlay::ProfileManager) {
                    self.close_profile_manager();
                } else {
                    self.overlay = None;
                }
                Vec::new()
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
                was_active,
            } => self.profile_deleted(request_id, profile_id, was_active),
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
                if !self.connection_matches(profile_id, generation) {
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
                self.connection.status = ConnectionStatus::Failed;
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
            Action::DisconnectCompleted { profile_id } => {
                if self.connection.profile_id == Some(profile_id) {
                    self.connection = ConnectionState::default();
                    self.explorer = ExplorerState::default();
                }
                Vec::new()
            }
            Action::ReplaceEditor(text) => {
                self.active_console_mut().editor.set_text(text);
                vec![Command::PersistWorkspace]
            }
            Action::InsertCharacter(character) => {
                self.active_console_mut().editor.insert(character);
                Vec::new()
            }
            Action::InsertNewline => {
                self.active_console_mut().editor.newline();
                Vec::new()
            }
            Action::Backspace => {
                self.active_console_mut().editor.backspace();
                Vec::new()
            }
            Action::Delete => {
                self.active_console_mut().editor.delete();
                Vec::new()
            }
            Action::MoveLeft => {
                self.active_console_mut().editor.move_left();
                Vec::new()
            }
            Action::MoveRight => {
                self.active_console_mut().editor.move_right();
                Vec::new()
            }
            Action::MoveUp => {
                self.active_console_mut().editor.move_up();
                Vec::new()
            }
            Action::MoveDown => {
                self.active_console_mut().editor.move_down();
                Vec::new()
            }
            Action::MoveHome => {
                self.active_console_mut().editor.move_home();
                Vec::new()
            }
            Action::MoveEnd => {
                self.active_console_mut().editor.move_end();
                Vec::new()
            }
            Action::EnterNormalMode => {
                self.active_console_mut().editor.mode = EditorMode::Normal;
                Vec::new()
            }
            Action::EnterInsertMode => {
                self.active_console_mut().editor.mode = EditorMode::Insert;
                Vec::new()
            }
            Action::EnterAppendMode => {
                let editor = &mut self.active_console_mut().editor;
                editor.move_right();
                editor.mode = EditorMode::Insert;
                Vec::new()
            }
            Action::OpenLineBelow => {
                let editor = &mut self.active_console_mut().editor;
                editor.move_end();
                editor.newline();
                editor.mode = EditorMode::Insert;
                Vec::new()
            }
            Action::RunActiveSql => self.run_active_sql(),
            Action::CancelActiveQuery => {
                let tab = self.active_console_mut();
                if tab.query_status != QueryStatus::Running {
                    return Vec::new();
                }
                tab.query_status = QueryStatus::Cancelled;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Cancelled,
                    message: "Query cancellation requested".to_owned(),
                });
                vec![Command::CancelQuery {
                    tab_id: tab.id,
                    generation: tab.generation,
                }]
            }
            Action::RefreshCatalog => {
                let (Some(profile_id), ConnectionStatus::Connected) =
                    (self.connection.profile_id, self.connection.status)
                else {
                    return Vec::new();
                };
                vec![Command::LoadCatalog {
                    profile_id,
                    generation: self.connection.generation,
                }]
            }
            Action::PreviewSelected => self.preview_selected(),
            Action::DdlSelected => self.ddl_selected(),
            Action::RequestConnect(profile_id) => self.request_connection(profile_id),
            Action::ConnectionSucceeded {
                profile_id,
                generation,
                server,
            } => {
                if !self.connection_matches(profile_id, generation) {
                    return Vec::new();
                }
                self.connection.status = ConnectionStatus::Connected;
                self.connection.server = Some(server);
                self.connection.error = None;
                if let Some(manager) = self.profile_manager.as_mut()
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
                if self.connection_matches(profile_id, generation) {
                    self.connection.status = ConnectionStatus::Failed;
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
                if self.connection_matches(profile_id, generation) {
                    self.explorer.set_nodes(nodes);
                }
                Vec::new()
            }
            Action::CatalogFailed {
                profile_id,
                generation,
                message,
            } => {
                if self.connection_matches(profile_id, generation) {
                    self.connection.error = Some(message);
                }
                Vec::new()
            }
            Action::QueryFinished {
                tab_id,
                generation,
                outcome,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return Vec::new();
                };
                if tab.generation != generation {
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
                Vec::new()
            }
            Action::QueryFailed {
                tab_id,
                generation,
                message,
            } => {
                let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return Vec::new();
                };
                if tab.generation != generation {
                    return Vec::new();
                }
                tab.query_status = QueryStatus::Failed;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Error,
                    message,
                });
                tab.result_view = ResultView::Output;
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
                tab.editor.set_text(sql);
                tab.editor.mode = EditorMode::Normal;
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
                tab.editor.set_text(ddl);
                tab.editor.mode = EditorMode::Normal;
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
                self.should_quit = true;
                vec![Command::Quit]
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
        let Some(manager) = self.idle_profile_manager_mut(ProfileManagerPage::ConfirmDelete) else {
            return Vec::new();
        };
        if blocked {
            manager.page = ProfileManagerPage::List;
            manager.message = Some("Cancel the running query before deleting this profile".into());
            return Vec::new();
        }
        let request_id = next_profile_request(manager);
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
        if connect && target_profile_id != self.connection.profile_id && self.has_running_query() {
            if let Some(manager) = self.editable_profile_manager_mut() {
                manager.message = Some("Cancel the running query before switching profiles".into());
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
            return if self.connection.profile_id == Some(profile_id)
                && self.connection.status == ConnectionStatus::Connecting
            {
                vec![Command::Disconnect { profile_id }]
            } else {
                Vec::new()
            };
        }
        if self.connection.profile_id != Some(profile_id) && self.has_running_query() {
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
        was_active: bool,
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
        if was_active || self.connection.profile_id == Some(profile_id) {
            vec![Command::Disconnect { profile_id }]
        } else {
            Vec::new()
        }
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
        if !self.profiles.iter().any(|profile| profile.id == profile_id)
            || (self.connection.profile_id != Some(profile_id) && self.has_running_query())
        {
            return Vec::new();
        }
        self.connection.generation += 1;
        self.connection.profile_id = Some(profile_id);
        self.connection.status = ConnectionStatus::Connecting;
        self.connection.server = None;
        self.connection.error = None;
        vec![Command::Connect {
            profile_id,
            generation: self.connection.generation,
        }]
    }

    fn has_running_query(&self) -> bool {
        self.tabs
            .iter()
            .any(|tab| tab.query_status == QueryStatus::Running)
    }

    fn run_active_sql(&mut self) -> Vec<Command> {
        let tab = self.active_console_mut();
        let sql = tab.editor.text();
        if sql.trim().is_empty() || tab.query_status == QueryStatus::Running {
            return Vec::new();
        }
        tab.generation += 1;
        tab.query_status = QueryStatus::Running;
        tab.output.push(OutputEntry {
            kind: OutputKind::Info,
            message: "Executing SQL".to_owned(),
        });
        vec![Command::RunQuery {
            tab_id: tab.id,
            generation: tab.generation,
            sql,
        }]
    }

    fn preview_selected(&mut self) -> Vec<Command> {
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
        tab.editor
            .set_text(format!("-- Loading preview for {schema}.{}", node.name));
        tab.editor.mode = EditorMode::Normal;
        let tab_id = tab.id;
        let generation = tab.generation;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.focus = Focus::Results;
        vec![Command::PreviewTable {
            tab_id,
            generation,
            schema,
            name: node.name,
        }]
    }

    fn ddl_selected(&mut self) -> Vec<Command> {
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
        self.active_tab = self.tabs.len() - 1;
        self.focus = Focus::Editor;
        vec![Command::LoadDdl {
            tab_id,
            generation,
            kind: node.kind,
            schema,
            name: node.name,
        }]
    }

    fn connection_matches(&self, profile_id: Uuid, generation: u64) -> bool {
        self.connection.profile_id == Some(profile_id) && self.connection.generation == generation
    }
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
        model::workspace::{Focus, Overlay, QueryStatus},
        profile::import_connection_url,
    };

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
            outcome: empty_outcome(),
        });
        assert!(app.active_console().outcome.is_none());

        app.update(Action::QueryFinished {
            tab_id,
            generation,
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
