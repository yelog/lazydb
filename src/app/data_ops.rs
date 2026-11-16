use chrono::Utc;
use uuid::Uuid;

use super::App;
use crate::{
    action::Command,
    agent::policy::{WriteCapability, WritePolicy, write_capability},
    commands::{CommandAvailability, CommandContext, CommandId},
    data_ops::{
        DataEndpoint, DataJobRecord, DataJobSpec, DataJobStatus, DataOperation,
        supports_native_data_operations,
    },
    model::{
        data_ops::{DataField, DataOperationAction, DataOperationDraft},
        execution_target::ExecutionTarget,
        explorer::ExplorerNodeId,
        workspace::{Focus, Overlay},
    },
    profile::DatabaseKind,
    security::sanitize_terminal_text,
};

impl App {
    pub fn set_data_operation_write_policy(&mut self, policy: WritePolicy) {
        self.data_operations.write_policy = policy;
    }

    pub(super) fn open_data_tasks_before_quit(&mut self) -> Vec<Command> {
        let commands = self.update_data_operation(DataOperationAction::OpenTasks);
        if let Some(index) = self
            .data_operations
            .records
            .iter()
            .position(|record| {
                self.data_operations.runs.contains_key(&record.id)
                    && !self.data_operations.cancelling.contains(&record.id)
            })
            .or_else(|| {
                self.data_operations
                    .records
                    .iter()
                    .position(|record| self.data_operations.runs.contains_key(&record.id))
            })
        {
            self.data_operations.selected = index;
        }
        self.data_operations.scroll = 0;
        self.omni = None;
        self.notify_warning(
            "Data tasks are active",
            "Wait for tasks to finish, or press c in Data Tasks to cancel and wait for cancellation before quitting again",
        );
        commands
    }

    fn visible_data_profile_ids(&self) -> std::collections::HashSet<Uuid> {
        self.profiles
            .iter()
            .filter(|profile| {
                profile.access == crate::profile::ProfileAccess::Global
                    || profile.access.contains_project(&self.project.root)
                    || self
                        .explorer
                        .normalized
                        .profiles
                        .get(&profile.id)
                        .is_some_and(|state| {
                            state.provenance == crate::model::explorer::ProfileProvenance::Session
                        })
            })
            .map(|profile| profile.id)
            .collect()
    }

    pub(crate) fn data_operation_command_context(&self) -> CommandContext {
        if self.focus == Focus::Explorer {
            let selected = self.explorer.selected_id();
            let profile_id = selected.and_then(ExplorerNodeId::profile_id);
            let catalog_id = match selected {
                Some(ExplorerNodeId::Catalog(id) | ExplorerNodeId::ProjectionGroup(id)) => {
                    Some(id.clone())
                }
                Some(ExplorerNodeId::Group { parent, .. }) => Some(parent.clone()),
                _ => None,
            };
            let target = profile_id
                .and_then(|id| self.profiles.iter().find(|profile| profile.id == id))
                .map(|profile| {
                    let mut target = ExecutionTarget::from_profile(profile);
                    if profile.kind != DatabaseKind::Sqlite
                        && let Some(database) =
                            catalog_id.as_ref().and_then(|id| id.native_path.first())
                    {
                        target.database = database.clone();
                    }
                    target
                });
            return CommandContext {
                profile_id,
                target,
                catalog_id,
                ..Default::default()
            };
        }
        if let Some(crate::model::tab::WorkspaceTab::Relation(tab)) = self.tabs.get(self.active_tab)
        {
            return CommandContext {
                profile_id: Some(tab.descriptor.key.profile_id),
                catalog_id: Some(tab.descriptor.key.object_id.clone()),
                tab_id: Some(tab.id),
                ..Default::default()
            };
        }
        let target = self
            .active_console_opt()
            .and_then(|console| console.execution_target.clone());
        CommandContext {
            profile_id: target.as_ref().map(|target| target.profile_id),
            target,
            ..Default::default()
        }
    }

    pub fn data_command_availability(
        &self,
        id: CommandId,
        context: &CommandContext,
    ) -> Option<CommandAvailability> {
        if !matches!(
            id,
            CommandId::BackupDatabase | CommandId::RestoreDatabase | CommandId::CopyDatabase
        ) {
            return None;
        }
        if id != CommandId::BackupDatabase && self.data_operations.write_policy == WritePolicy::Deny
        {
            return Some(CommandAvailability::Disabled(
                "Read-only mode forbids restore and copy",
            ));
        }
        let profile_id = context
            .catalog_id
            .as_ref()
            .map(|id| id.profile_id())
            .or_else(|| context.target.as_ref().map(|target| target.profile_id))
            .or(context.profile_id);
        let profile =
            profile_id.and_then(|id| self.profiles.iter().find(|profile| profile.id == id));
        if let Some(profile) = profile {
            if !supports_native_data_operations(profile.kind) {
                return Some(CommandAvailability::Disabled(
                    "Native data operations support PostgreSQL, MySQL, MariaDB and SQLite",
                ));
            }
            if id == CommandId::RestoreDatabase
                && write_capability(profile, self.data_operations.write_policy)
                    != WriteCapability::Allowed
            {
                return Some(CommandAvailability::Disabled("Target profile is read-only"));
            }
        } else if !self
            .profiles
            .iter()
            .any(|profile| supports_native_data_operations(profile.kind))
        {
            return Some(CommandAvailability::Disabled(
                "No supported database profile is available",
            ));
        }
        Some(CommandAvailability::Ready)
    }

    fn data_endpoint_for_context(&self, context: &CommandContext) -> Option<DataEndpoint> {
        let profile_id = context
            .catalog_id
            .as_ref()
            .map(|id| id.profile_id())
            .or_else(|| context.target.as_ref().map(|target| target.profile_id))
            .or(context.profile_id)?;
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)?;
        let mut endpoint = DataEndpoint::from_profile(profile);
        if profile.kind != DatabaseKind::Sqlite {
            if let Some(database) = context
                .catalog_id
                .as_ref()
                .and_then(|id| id.native_path.first())
                .or_else(|| context.target.as_ref().map(|target| &target.database))
            {
                endpoint.database.clone_from(database);
            }
        }
        Some(endpoint)
    }

    fn validate_data_spec(&self, spec: &DataJobSpec) -> Result<(), String> {
        for endpoint in [&spec.source, &spec.destination].into_iter().flatten() {
            let profile = self
                .profiles
                .iter()
                .find(|profile| profile.id == endpoint.profile_id)
                .ok_or("A selected profile is no longer available")?;
            if !supports_native_data_operations(profile.kind) {
                return Err("This database engine does not support native data operations".into());
            }
        }
        if spec.operation.writes_database() {
            let destination = spec.destination.as_ref().ok_or("Select a target profile")?;
            let profile = self
                .profiles
                .iter()
                .find(|profile| profile.id == destination.profile_id)
                .ok_or("The target profile is no longer available")?;
            if write_capability(profile, self.data_operations.write_policy)
                != WriteCapability::Allowed
            {
                return Err("Read-only policy forbids writing to the target profile".into());
            }
            if let Some(source) = &spec.source {
                let source_profile = self
                    .profiles
                    .iter()
                    .find(|profile| profile.id == source.profile_id);
                if source_profile.is_none_or(|source| source.kind != profile.kind) {
                    return Err(
                        "Copy requires the exact same database engine at both endpoints".into(),
                    );
                }
                if source == destination {
                    return Err("The target must be a NEW database / file".into());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn update_data_operation(&mut self, action: DataOperationAction) -> Vec<Command> {
        use DataOperationAction as A;
        match action {
            A::Open { operation, context } => {
                let command = match operation {
                    DataOperation::Backup => CommandId::BackupDatabase,
                    DataOperation::Restore => CommandId::RestoreDatabase,
                    DataOperation::Copy => CommandId::CopyDatabase,
                };
                if let Some(CommandAvailability::Disabled(reason)) =
                    self.data_command_availability(command, &context)
                {
                    self.notify_warning("Data operations", reason);
                    return Vec::new();
                }
                let commands = self.discard_data_plan();
                let endpoint = self.data_endpoint_for_context(&context);
                let mut draft = DataOperationDraft::new(operation, endpoint);
                if draft.source_profile_id.is_none() {
                    draft.focus = if operation == DataOperation::Restore {
                        DataField::TargetProfile
                    } else {
                        DataField::SourceProfile
                    };
                } else if operation == DataOperation::Copy
                    && draft.target_profile_id.is_some_and(|id| {
                        self.profiles
                            .iter()
                            .find(|profile| profile.id == id)
                            .is_none_or(|profile| {
                                write_capability(profile, self.data_operations.write_policy)
                                    != WriteCapability::Allowed
                            })
                    })
                {
                    draft.target_profile_id = None;
                    draft.focus = DataField::TargetProfile;
                }
                self.data_operations.draft = Some(draft);
                self.overlay = Some(Overlay::DataOperation);
                commands
            }
            A::OpenTasks | A::RefreshTasks => {
                let visible = self.visible_data_profile_ids();
                self.data_operations.records.retain(|record| {
                    self.data_operations.runs.contains_key(&record.id)
                        || record
                            .plan
                            .spec
                            .source
                            .iter()
                            .chain(record.plan.spec.destination.iter())
                            .all(|endpoint| visible.contains(&endpoint.profile_id))
                });
                self.data_operations.selected = self
                    .data_operations
                    .selected
                    .min(self.data_operations.records.len().saturating_sub(1));
                let request_id = Uuid::new_v4();
                self.data_operations.history_request = Some(request_id);
                self.data_operations.history_error = None;
                self.overlay = Some(Overlay::DataTasks);
                vec![Command::LoadDataTasks { request_id }]
            }
            A::TasksLoaded { request_id, result } => {
                if self.data_operations.history_request != Some(request_id) {
                    return Vec::new();
                }
                self.data_operations.history_request = None;
                let visible = self.visible_data_profile_ids();
                match result {
                    Ok(records) => {
                        for record in records {
                            if !self.data_operations.runs.contains_key(&record.id)
                                && record
                                    .plan
                                    .spec
                                    .source
                                    .iter()
                                    .chain(record.plan.spec.destination.iter())
                                    .all(|endpoint| visible.contains(&endpoint.profile_id))
                            {
                                self.data_operations.merge_record(record);
                            }
                        }
                    }
                    Err(message) => {
                        self.data_operations.history_error = Some(sanitize_terminal_text(&message));
                    }
                }
                Vec::new()
            }
            A::PlanReady { request_id, result } => {
                let Some(draft) = self
                    .data_operations
                    .draft
                    .as_mut()
                    .filter(|draft| draft.pending_plan == Some(request_id))
                else {
                    return vec![Command::CancelDataOperation { request_id }];
                };
                if draft.plan.is_some() {
                    return Vec::new();
                }
                match result {
                    Ok(plan)
                        if plan.spec.operation == draft.operation
                            && plan
                                .spec
                                .source
                                .as_ref()
                                .map(|endpoint| endpoint.profile_id)
                                == (draft.operation != DataOperation::Restore)
                                    .then_some(draft.source_profile_id)
                                    .flatten()
                            && plan
                                .spec
                                .destination
                                .as_ref()
                                .map(|endpoint| endpoint.profile_id)
                                == draft
                                    .operation
                                    .writes_database()
                                    .then_some(draft.target_profile_id)
                                    .flatten() =>
                    {
                        draft.plan = Some(*plan);
                        draft.scroll = 0;
                    }
                    Ok(_) => {
                        draft.pending_plan = None;
                        draft.error =
                            Some("The plan does not match this draft; review again".into());
                        return vec![Command::CancelDataOperation { request_id }];
                    }
                    Err(message) => {
                        draft.pending_plan = None;
                        draft.error = Some(sanitize_terminal_text(&message));
                    }
                }
                Vec::new()
            }
            A::JobUpdated { request_id, record } => {
                if self.data_operations.runs.get(&record.id) != Some(&request_id)
                    || record.plan.id != record.id
                    || self
                        .data_operations
                        .records
                        .iter()
                        .find(|entry| entry.id == record.id)
                        .is_none_or(|entry| {
                            entry.plan != record.plan || entry.updated_at > record.updated_at
                        })
                {
                    return Vec::new();
                }
                if record.status.is_terminal() {
                    self.data_operations.runs.remove(&record.id);
                    self.data_operations.cancelling.remove(&record.id);
                    let message =
                        format!("{}: {}", record.plan.spec.operation.label(), record.message);
                    if record.status.is_success() {
                        self.notify_success("Data task", sanitize_terminal_text(&message));
                    } else {
                        self.notify_warning("Data task", sanitize_terminal_text(&message));
                    }
                }
                self.data_operations.merge_record(*record);
                Vec::new()
            }
            A::JobFailed {
                request_id,
                job_id,
                message,
            } => {
                if self.data_operations.runs.get(&job_id) != Some(&request_id) {
                    return Vec::new();
                }
                self.data_operations.runs.remove(&job_id);
                self.data_operations.cancelling.remove(&job_id);
                if let Some(record) = self
                    .data_operations
                    .records
                    .iter_mut()
                    .find(|record| record.id == job_id)
                {
                    record.status = DataJobStatus::Failed;
                    record.message = "Data task failed".into();
                    record.error = Some(sanitize_terminal_text(&message));
                    record.updated_at = Utc::now();
                }
                self.notify_warning("Data task failed", sanitize_terminal_text(&message));
                Vec::new()
            }
            A::Close => {
                let commands = if self.overlay == Some(Overlay::DataOperation) {
                    let commands = self.discard_data_plan();
                    self.data_operations.draft = None;
                    commands
                } else {
                    Vec::new()
                };
                self.overlay = None;
                commands
            }
            A::TaskMove(delta) if self.overlay == Some(Overlay::DataTasks) => {
                self.data_operations.selected = self
                    .data_operations
                    .selected
                    .saturating_add_signed(delta)
                    .min(self.data_operations.records.len().saturating_sub(1));
                self.data_operations.scroll = 0;
                Vec::new()
            }
            A::TaskSelect(index) if self.overlay == Some(Overlay::DataTasks) => {
                self.data_operations.selected =
                    index.min(self.data_operations.records.len().saturating_sub(1));
                self.data_operations.scroll = 0;
                Vec::new()
            }
            A::CancelTask if self.overlay == Some(Overlay::DataTasks) => {
                let Some(record) = self.data_operations.selected_record() else {
                    return Vec::new();
                };
                let job_id = record.id;
                if record.status.is_terminal() {
                    return Vec::new();
                }
                let Some(request_id) = self.data_operations.runs.get(&job_id).copied() else {
                    return Vec::new();
                };
                self.data_operations.cancelling.insert(job_id);
                vec![Command::CancelDataOperation { request_id }]
            }
            A::RerunTask if self.overlay == Some(Overlay::DataTasks) => {
                let Some(record) = self
                    .data_operations
                    .selected_record()
                    .filter(|record| record.status.is_terminal())
                else {
                    return Vec::new();
                };
                self.data_operations.draft = Some(DataOperationDraft::from_spec(&record.plan.spec));
                self.overlay = Some(Overlay::DataOperation);
                Vec::new()
            }
            A::Scroll(delta) => {
                if self.overlay == Some(Overlay::DataTasks) {
                    self.data_operations.scroll = self
                        .data_operations
                        .scroll
                        .saturating_add_signed(delta)
                        .min(10_000);
                } else if self.overlay == Some(Overlay::DataOperation)
                    && let Some(draft) = self.data_operations.draft.as_mut()
                {
                    draft.scroll = draft.scroll.saturating_add_signed(delta).min(10_000);
                }
                Vec::new()
            }
            A::Review if self.overlay == Some(Overlay::DataOperation) => {
                let Some(draft) = self
                    .data_operations
                    .draft
                    .as_ref()
                    .filter(|draft| draft.plan.is_none() && draft.pending_plan.is_none())
                else {
                    return Vec::new();
                };
                let spec = draft.spec().and_then(|spec| {
                    self.validate_data_spec(&spec)?;
                    Ok(spec)
                });
                match spec {
                    Ok(spec) => {
                        let request_id = Uuid::new_v4();
                        let draft = self.data_operations.draft.as_mut().unwrap();
                        draft.pending_plan = Some(request_id);
                        draft.error = None;
                        vec![Command::PlanDataOperation {
                            request_id,
                            spec,
                            policy: self.data_operations.write_policy,
                        }]
                    }
                    Err(message) => {
                        self.data_operations.draft.as_mut().unwrap().error = Some(message);
                        Vec::new()
                    }
                }
            }
            A::Execute if self.overlay == Some(Overlay::DataOperation) => {
                let Some(plan) = self
                    .data_operations
                    .draft
                    .as_ref()
                    .and_then(|draft| draft.plan.clone())
                else {
                    return Vec::new();
                };
                if let Err(message) = self.validate_data_spec(&plan.spec) {
                    self.data_operations.draft.as_mut().unwrap().error = Some(message);
                    return Vec::new();
                }
                let request_id = Uuid::new_v4();
                let now = Utc::now();
                self.data_operations.runs.insert(plan.id, request_id);
                self.data_operations.merge_record(DataJobRecord {
                    schema_version: 1,
                    id: plan.id,
                    plan: plan.clone(),
                    status: DataJobStatus::Queued,
                    created_at: now,
                    updated_at: now,
                    message: "Queued".into(),
                    bytes: None,
                    log: Vec::new(),
                    outcome: None,
                    error: None,
                    target_may_be_partial: false,
                });
                self.data_operations.selected = self
                    .data_operations
                    .records
                    .iter()
                    .position(|record| record.id == plan.id)
                    .unwrap_or_default();
                self.data_operations.draft = None;
                self.data_operations.scroll = 0;
                self.overlay = Some(Overlay::DataTasks);
                vec![Command::RunDataOperation {
                    request_id,
                    plan,
                    policy: self.data_operations.write_policy,
                }]
            }
            A::Back if self.overlay == Some(Overlay::DataOperation) => self.discard_data_plan(),
            A::FocusNext(forward) if self.overlay == Some(Overlay::DataOperation) => {
                if let Some(draft) = self
                    .data_operations
                    .draft
                    .as_mut()
                    .filter(|draft| draft.plan.is_none())
                {
                    let fields = draft.fields();
                    let index = fields
                        .iter()
                        .position(|field| *field == draft.focus)
                        .unwrap_or_default();
                    draft.focus = fields[(index as isize + if forward { 1 } else { -1 })
                        .rem_euclid(fields.len() as isize)
                        as usize];
                }
                Vec::new()
            }
            A::FocusField(field) if self.overlay == Some(Overlay::DataOperation) => {
                if let Some(draft) = self
                    .data_operations
                    .draft
                    .as_mut()
                    .filter(|draft| draft.plan.is_none() && draft.fields().contains(&field))
                {
                    draft.focus = field;
                }
                Vec::new()
            }
            A::Edit(edit) if self.overlay == Some(Overlay::DataOperation) => {
                let Some(draft) = self
                    .data_operations
                    .draft
                    .as_mut()
                    .filter(|draft| draft.plan.is_none())
                else {
                    return Vec::new();
                };
                let changed = draft.input_mut().is_some_and(|input| input.apply(edit));
                if changed {
                    self.discard_data_plan()
                } else {
                    Vec::new()
                }
            }
            A::Paste(value) if self.overlay == Some(Overlay::DataOperation) => {
                let Some(draft) = self
                    .data_operations
                    .draft
                    .as_mut()
                    .filter(|draft| draft.plan.is_none())
                else {
                    return Vec::new();
                };
                if let Some(input) = draft.input_mut() {
                    input.paste(sanitize_terminal_text(&value).replace(['\n', '\r', '\t'], ""));
                    return self.discard_data_plan();
                }
                Vec::new()
            }
            A::CycleProfile(delta) if self.overlay == Some(Overlay::DataOperation) => {
                let Some(draft) = self
                    .data_operations
                    .draft
                    .as_ref()
                    .filter(|draft| draft.plan.is_none())
                else {
                    return Vec::new();
                };
                let source = draft.focus == DataField::SourceProfile;
                if !source && draft.focus != DataField::TargetProfile {
                    return Vec::new();
                }
                let source_kind = draft
                    .source_profile_id
                    .and_then(|id| self.profiles.iter().find(|profile| profile.id == id))
                    .map(|profile| profile.kind);
                let profiles = self
                    .profiles
                    .iter()
                    .filter(|profile| {
                        supports_native_data_operations(profile.kind)
                            && (source
                                || write_capability(profile, self.data_operations.write_policy)
                                    == WriteCapability::Allowed)
                            && (source
                                || draft.operation != DataOperation::Copy
                                || source_kind == Some(profile.kind))
                    })
                    .collect::<Vec<_>>();
                if profiles.is_empty() {
                    self.data_operations.draft.as_mut().unwrap().error =
                        Some("No eligible profile is available".into());
                    return Vec::new();
                }
                let current = if source {
                    draft.source_profile_id
                } else {
                    draft.target_profile_id
                };
                let index = profiles
                    .iter()
                    .position(|profile| Some(profile.id) == current);
                let index = index
                    .map(|index| {
                        (index as isize + delta).rem_euclid(profiles.len() as isize) as usize
                    })
                    .unwrap_or(if delta < 0 { profiles.len() - 1 } else { 0 });
                let profile = profiles[index];
                let endpoint = DataEndpoint::from_profile(profile);
                let draft = self.data_operations.draft.as_mut().unwrap();
                if source {
                    draft.source_profile_id = Some(profile.id);
                    draft.source_database.set(endpoint.database);
                    if draft.operation == DataOperation::Copy {
                        draft.target_profile_id = None;
                    }
                } else {
                    draft.target_profile_id = Some(profile.id);
                }
                self.discard_data_plan()
            }
            _ => Vec::new(),
        }
    }

    fn discard_data_plan(&mut self) -> Vec<Command> {
        let Some(draft) = self.data_operations.draft.as_mut() else {
            return Vec::new();
        };
        draft.plan = None;
        draft.error = None;
        draft.scroll = 0;
        draft
            .pending_plan
            .take()
            .map(|request_id| Command::CancelDataOperation { request_id })
            .into_iter()
            .collect()
    }
}
