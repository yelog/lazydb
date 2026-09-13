#![allow(clippy::type_complexity)]

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use uuid::Uuid;

use crate::{
    action::{Action, Command, ProfileAccessChange, ProfileOrganizationMutation},
    app::App,
    cli::{Cli, ColorMode, MouseMode},
    db::{
        DatabaseConnection, DatabaseError,
        catalog::{CatalogDiscovery, DiscoveredDatabase},
        transaction::TransactionRequest,
    },
    input::{
        keymap::{Keymap, map_paste},
        mouse::map_mouse,
    },
    model::{
        execution_target::ExecutionTarget,
        profile_manager::{CredentialUpdate, ProfileChange, ProfileSubmission},
        workspace::ConnectionIdentity,
    },
    persistence::{
        credentials::CredentialResolver,
        local_credentials::LocalCredentialStore,
        paths::AppPaths,
        profiles::ProfileStore,
        secrets::{
            NativeSecretStore, SecretStore, SecretStoreAvailability, SecretStoreError, keyring_ref,
            profile_id_from_ref,
        },
        workspace::WorkspaceStore,
    },
    profile::{ConnectionProfile, CredentialPolicy, ProfileCollection, import_connection_url},
    security::sanitize_terminal_text,
    terminal::TerminalSession,
    ui::{self, UiState, theme::Theme},
};
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, MouseEventKind};
use futures_util::StreamExt;
use secrecy::SecretString;
use tokio::{
    sync::{Mutex, mpsc},
    task::{self, JoinHandle},
    time::sleep,
    time::{MissedTickBehavior, interval, timeout},
};

pub(crate) mod connections;
pub(crate) mod transaction;

use connections::{ConnectionAttempts, ConnectionKey};
use transaction::ForcedCloseHandle;

#[derive(Clone, Debug)]
enum ThemeUpdate {
    Theme(Theme),
    Error(String),
}

fn apply_theme_update(theme: &mut Theme, update: ThemeUpdate) -> (bool, Option<String>) {
    match update {
        ThemeUpdate::Theme(next) => {
            let changed = *theme != next;
            *theme = next;
            (changed, None)
        }
        ThemeUpdate::Error(error) => (false, Some(error)),
    }
}

async fn poll_external_theme(
    path: std::path::PathBuf,
    sender: tokio::sync::watch::Sender<ThemeUpdate>,
) {
    let mut source = crate::ui::theme::external::ExternalThemeSource::new(path);
    let mut ticker = interval(Duration::from_millis(250));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let update = match source.check().await {
            crate::ui::theme::external::SourceOutcome::ThemeChanged(theme) => {
                ThemeUpdate::Theme(theme)
            }
            crate::ui::theme::external::SourceOutcome::Error(error) => {
                ThemeUpdate::Error(error.to_string())
            }
            crate::ui::theme::external::SourceOutcome::Unchanged => continue,
        };
        if sender.send(update).is_err() {
            break;
        }
    }
}

fn clipboard_write_failure_message() -> String {
    "Clipboard unavailable".to_owned()
}

#[derive(Clone, Debug)]
struct ActiveConnection {
    database: DatabaseConnection,
}

struct CatalogMutationConnection {
    database: DatabaseConnection,
    owned: bool,
}

impl CatalogMutationConnection {
    async fn close_if_owned(self) {
        if self.owned {
            self.database.close().await;
        }
    }
}

#[derive(Clone)]
struct ProfileRegistry {
    groups: Vec<crate::profile::ConnectionGroup>,
    order: Vec<Uuid>,
    profiles: HashMap<Uuid, ConnectionProfile>,
    revisions: HashMap<Uuid, u64>,
    persisted: HashSet<Uuid>,
    session_secrets: HashMap<Uuid, SecretString>,
    startup_password_profile: Option<Uuid>,
    startup_password: Option<SecretString>,
}

struct ManualTransactionEntry {
    connection: ConnectionIdentity,
    target: ExecutionTarget,
    transaction_id: Uuid,
    transaction_generation: u64,
    request_sender: tokio::sync::mpsc::UnboundedSender<crate::db::transaction::TransactionRequest>,
    worker_handle: JoinHandle<crate::db::transaction::WorkerDisposition>,
    cancellation_sender: Option<tokio::sync::oneshot::Sender<()>>,
    forced_close_handle: ForcedCloseHandle,
}

struct WorkspaceSaveQueue {
    store: WorkspaceStore,
    pending: Option<(u64, crate::persistence::workspace::WorkspaceSnapshot)>,
    failed: Option<(u64, crate::persistence::workspace::WorkspaceSnapshot)>,
    active: Option<JoinHandle<()>>,
    next_revision: u64,
    acknowledged_revision: u64,
    flushing: Option<u64>,
}

impl WorkspaceSaveQueue {
    fn new(store: WorkspaceStore) -> Self {
        Self {
            store,
            pending: None,
            failed: None,
            active: None,
            next_revision: 0,
            acknowledged_revision: 0,
            flushing: None,
        }
    }

    fn offer(&mut self, revision: u64, snapshot: crate::persistence::workspace::WorkspaceSnapshot) {
        self.next_revision = self.next_revision.max(revision);
        self.pending = Some((revision, snapshot));
    }

    fn flush(&mut self, revision: u64, sender: mpsc::UnboundedSender<Action>) {
        self.flushing = Some(revision);
        self.poll(sender);
    }

    fn poll(&mut self, sender: mpsc::UnboundedSender<Action>) {
        if self.active.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        if let Some(active) = self.active.take()
            && !active.is_finished()
        {
            self.active = Some(active);
            return;
        }
        let Some((revision, snapshot)) = self.pending.take() else {
            if let Some(revision) = self.flushing
                && self.acknowledged_revision >= revision
            {
                let _ = sender.send(Action::WorkspaceSaveFlushed { revision });
                self.flushing = None;
            }
            return;
        };
        let store = self.store.clone();
        let retry_snapshot = snapshot.clone();
        self.failed = Some((revision, retry_snapshot));
        self.active = Some(tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || store.save(&snapshot))
                .await
                .unwrap_or_else(|error| {
                    Err(crate::persistence::workspace::WorkspaceError::Invalid(
                        format!("workspace save worker failed: {error}"),
                    ))
                });
            let action = match result {
                Ok(()) => Action::WorkspaceSaveSucceeded { revision },
                Err(error) => Action::WorkspaceSaveFailed {
                    revision,
                    message: error.to_string(),
                },
            };
            let _ = sender.send(action);
        }));
    }

    fn complete(&mut self, revision: u64, succeeded: bool, sender: mpsc::UnboundedSender<Action>) {
        // The worker sends its completion action only after the blocking save
        // returns. Dropping this completed wrapper releases the queue slot even
        // if Tokio has not observed the outer task as finished yet.
        self.active.take();
        if succeeded {
            self.acknowledged_revision = self.acknowledged_revision.max(revision);
        }
        if succeeded
            && self
                .failed
                .as_ref()
                .is_some_and(|(failed_revision, _)| *failed_revision == revision)
        {
            self.failed = None;
        }
        if self.next_revision < revision {
            return;
        }
        self.poll(sender);
    }

    fn retry(&mut self, sender: mpsc::UnboundedSender<Action>) {
        if self.pending.is_none() {
            self.pending = self.failed.take();
        }
        self.poll(sender);
    }
}

pub struct Runtime {
    registry: Arc<Mutex<ProfileRegistry>>,
    profile_store: ProfileStore,
    workspace_store: Option<WorkspaceStore>,
    workspace_mutation: Arc<Mutex<()>>,
    workspace_save: Option<WorkspaceSaveQueue>,
    secret_store: Arc<dyn SecretStore>,
    local_credential_store: LocalCredentialStore,
    profile_mutation: Arc<Mutex<()>>,
    event_sender: mpsc::UnboundedSender<Action>,
    connection: Arc<Mutex<HashMap<ConnectionKey, ActiveConnection>>>,
    connection_attempts: Arc<StdMutex<ConnectionAttempts>>,
    query_tasks: HashMap<(Uuid, u64), JoinHandle<()>>,
    catalog_drop_plan_tasks: HashMap<(ConnectionIdentity, u64), JoinHandle<()>>,
    catalog_drop_execute_tasks: HashMap<(ConnectionIdentity, u64), JoinHandle<()>>,
    catalog_mutation_tasks: HashMap<(ConnectionIdentity, u64), JoinHandle<()>>,
    relation_tasks: HashMap<crate::model::relation::RelationRequest, JoinHandle<()>>,
    dashboard_metric_tasks: HashMap<(Uuid, u64), JoinHandle<()>>,
    dashboard_metadata_tasks: HashMap<(Uuid, u64), JoinHandle<()>>,
    dashboard_process_tasks: HashMap<(Uuid, u64), JoinHandle<()>>,
    known_relations: Arc<StdMutex<HashSet<(ConnectionIdentity, crate::db::catalog::CatalogId)>>>,
    known_relation_targets: Arc<
        StdMutex<
            HashMap<
                (ConnectionIdentity, crate::db::catalog::CatalogId),
                crate::db::catalog::CatalogTarget,
            >,
        >,
    >,
    latest_catalog_requests: Arc<
        StdMutex<
            HashMap<
                (ConnectionIdentity, crate::db::catalog::CatalogTarget),
                crate::db::catalog::CatalogRequestKey,
            >,
        >,
    >,
    background_tasks: Vec<JoinHandle<()>>,
    update_check_task: Option<JoinHandle<()>>,
    update_install_task: Option<JoinHandle<()>>,
    update_startup_task: Option<JoinHandle<()>>,
    profile_tasks: Vec<JoinHandle<()>>,
    completion_tasks: HashMap<Uuid, JoinHandle<()>>,
    diagnostic_tasks: HashMap<Uuid, JoinHandle<()>>,
    catalog_search_tasks: HashMap<
        (crate::action::CatalogSearchOwner, u64),
        (ConnectionIdentity, u64, JoinHandle<()>),
    >,
    manual_transactions: HashMap<Uuid, ManualTransactionEntry>,
    relation_transactions: HashMap<Uuid, ManualTransactionEntry>,
    relation_mutation_blocked: Arc<StdMutex<HashSet<(Uuid, ConnectionIdentity)>>>,
    clipboard: crate::config::ClipboardConfig,
    history_recorder: Option<crate::history::HistoryRecorder>,
    history_store: Option<crate::persistence::sql_history::HistoryStore>,
    history_executions: HashMap<(Uuid, u64), Uuid>,
}

#[derive(Debug)]
pub enum RunOutcome {
    Exit,
    Restart { executable: std::path::PathBuf },
}

impl Runtime {
    pub fn new(
        profiles: Vec<ConnectionProfile>,
        persisted: HashSet<Uuid>,
        session_secrets: HashMap<Uuid, SecretString>,
        startup_password: Option<(Uuid, SecretString)>,
        profile_store: ProfileStore,
        secret_store: Arc<dyn SecretStore>,
        event_sender: mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self::new_with_collection(
            ProfileCollection::from(profiles),
            persisted,
            session_secrets,
            startup_password,
            profile_store,
            secret_store,
            event_sender,
        )
    }

    pub fn new_with_collection(
        collection: ProfileCollection,
        persisted: HashSet<Uuid>,
        session_secrets: HashMap<Uuid, SecretString>,
        startup_password: Option<(Uuid, SecretString)>,
        profile_store: ProfileStore,
        secret_store: Arc<dyn SecretStore>,
        event_sender: mpsc::UnboundedSender<Action>,
    ) -> Self {
        let profiles = collection.profiles;
        let local_credential_store = local_credential_store_for(&profile_store);
        let mut order = Vec::with_capacity(profiles.len());
        let mut profiles_by_id = HashMap::with_capacity(profiles.len());
        for profile in profiles {
            if let std::collections::hash_map::Entry::Vacant(entry) =
                profiles_by_id.entry(profile.id)
            {
                order.push(profile.id);
                entry.insert(profile);
            }
        }
        let revisions = profiles_by_id
            .keys()
            .map(|profile_id| (*profile_id, 0))
            .collect();
        let (startup_password_profile, startup_password) = startup_password
            .map(|(profile_id, password)| (Some(profile_id), Some(password)))
            .unwrap_or((None, None));
        Self {
            registry: Arc::new(Mutex::new(ProfileRegistry {
                groups: collection.groups,
                order,
                profiles: profiles_by_id,
                revisions,
                persisted,
                session_secrets,
                startup_password_profile,
                startup_password,
            })),
            profile_store,
            workspace_store: None,
            workspace_mutation: Arc::new(Mutex::new(())),
            workspace_save: None,
            secret_store,
            local_credential_store,
            profile_mutation: Arc::new(Mutex::new(())),
            event_sender,
            connection: Arc::new(Mutex::new(HashMap::new())),
            connection_attempts: Arc::new(StdMutex::new(ConnectionAttempts::default())),
            query_tasks: HashMap::new(),
            catalog_drop_plan_tasks: HashMap::new(),
            catalog_drop_execute_tasks: HashMap::new(),
            catalog_mutation_tasks: HashMap::new(),
            relation_tasks: HashMap::new(),
            dashboard_metric_tasks: HashMap::new(),
            dashboard_metadata_tasks: HashMap::new(),
            dashboard_process_tasks: HashMap::new(),
            known_relations: Arc::new(StdMutex::new(HashSet::new())),
            known_relation_targets: Arc::new(StdMutex::new(HashMap::new())),
            latest_catalog_requests: Arc::new(StdMutex::new(HashMap::new())),
            background_tasks: Vec::new(),
            update_check_task: None,
            update_install_task: None,
            update_startup_task: None,
            profile_tasks: Vec::new(),
            completion_tasks: HashMap::new(),
            diagnostic_tasks: HashMap::new(),
            catalog_search_tasks: HashMap::new(),
            manual_transactions: HashMap::new(),
            relation_transactions: HashMap::new(),
            relation_mutation_blocked: Arc::new(StdMutex::new(HashSet::new())),
            clipboard: crate::config::ClipboardConfig {
                backend: crate::config::ClipboardBackend::System,
                max_bytes: 1_000_000,
            },
            history_recorder: None,
            history_store: None,
            history_executions: HashMap::new(),
        }
    }

    pub fn set_history_recorder(&mut self, recorder: crate::history::HistoryRecorder) {
        self.history_recorder = Some(recorder);
    }

    pub fn set_history_store(&mut self, store: crate::persistence::sql_history::HistoryStore) {
        self.history_store = Some(store);
    }

    pub fn set_clipboard_config(&mut self, clipboard: crate::config::ClipboardConfig) {
        self.clipboard = clipboard;
    }

    pub fn set_workspace_store(&mut self, store: WorkspaceStore) {
        self.workspace_save = Some(WorkspaceSaveQueue::new(store.clone()));
        self.workspace_store = Some(store);
    }

    pub fn dispatch(&mut self, command: Command) {
        self.query_tasks.retain(|_, task| !task.is_finished());
        self.history_executions
            .retain(|key, _| self.query_tasks.contains_key(key));
        self.catalog_drop_plan_tasks
            .retain(|_, task| !task.is_finished());
        self.catalog_drop_execute_tasks
            .retain(|_, task| !task.is_finished());
        self.catalog_mutation_tasks
            .retain(|_, task| !task.is_finished());
        self.relation_tasks.retain(|_, task| !task.is_finished());
        self.dashboard_metric_tasks
            .retain(|_, task| !task.is_finished());
        self.dashboard_metadata_tasks
            .retain(|_, task| !task.is_finished());
        self.dashboard_process_tasks
            .retain(|_, task| !task.is_finished());
        self.background_tasks.retain(|task| !task.is_finished());
        self.profile_tasks.retain(|task| !task.is_finished());
        match command {
            Command::CheckSecretStoreAvailability => self.check_secret_store_availability(),
            Command::TestProfile {
                request_id,
                submission,
            } => self.test_profile(request_id, submission),
            Command::DiscoverProfileCatalog {
                request_id,
                submission,
            } => self.discover_profile_catalog(request_id, submission),
            Command::SaveProfile {
                request_id,
                submission,
                connect,
            } => self.save_profile(request_id, submission, connect),
            Command::DeleteProfile {
                request_id,
                profile_id,
            } => self.delete_profile(request_id, profile_id),
            Command::UpdateProfileAccess {
                request_id,
                profile_id,
                change,
            } => self.update_profile_access(request_id, profile_id, change),
            Command::UpdateProfileOrganization {
                request_id,
                mutation,
            } => self.update_profile_organization(request_id, mutation),
            Command::Disconnect { connection } => self.disconnect(connection),
            Command::Connect {
                profile_id,
                generation,
                target,
            } => {
                self.connect(profile_id, generation, target);
            }
            Command::LoadCatalogPage(request) => self.load_catalog_page(request),
            Command::ResolveCatalogRelation {
                connection,
                catalog_epoch,
                request_id,
                relation,
                scope,
            } => self.resolve_catalog_relation(
                connection,
                catalog_epoch,
                request_id,
                relation,
                scope,
            ),
            Command::ReconcileCatalogRelation {
                connection,
                old_relation,
                new_relation,
            } => self.reconcile_catalog_relation(connection, old_relation, new_relation),
            Command::LoadCatalogObjectDefinition(request) => {
                self.load_catalog_object_definition(request)
            }
            Command::LoadCatalogOwnerContext(request) => self.load_catalog_owner_context(request),
            Command::SearchCatalog { owner, request } => self.search_catalog(owner, request),
            Command::CancelCatalogSearch {
                owner,
                session_id,
                generation,
            } => {
                let key = (owner, session_id);
                if self
                    .catalog_search_tasks
                    .get(&key)
                    .is_some_and(|(_, current_generation, _)| *current_generation == generation)
                    && let Some((_, _, task)) = self.catalog_search_tasks.remove(&key)
                {
                    task.abort();
                }
            }
            Command::PlanCatalogDrop(request) => self.plan_catalog_drop(request),
            Command::ExecuteCatalogDrop(plan) => self.execute_catalog_drop(plan),
            Command::PlanCatalogMutation {
                request,
                draft,
                baseline,
            } => self.plan_catalog_mutation(request, draft, baseline),
            Command::ExecuteCatalogMutation(plan) => self.execute_catalog_mutation(plan),
            Command::LoadRelationPreview(request) | Command::LoadRelationDdl(request) => {
                self.load_relation(request)
            }
            Command::CancelRelationRequest(request) => {
                if let Some(task) = self.relation_tasks.remove(&request) {
                    task.abort();
                }
            }
            Command::RunQuery {
                connection,
                target,
                tab_id,
                generation,
                sql,
            } => self.run_query(connection, target, tab_id, generation, sql),
            Command::LoadSqlHistory {
                generation,
                request,
            } => self.load_sql_history(generation, request),
            Command::LoadDashboardMetrics {
                tab_id,
                tab_generation,
                connection,
            } => self.load_dashboard_metrics(tab_id, tab_generation, connection),
            Command::LoadDashboardMetadata {
                tab_id,
                tab_generation,
                connection,
            } => self.load_dashboard_metadata(tab_id, tab_generation, connection),
            Command::LoadDashboardProcesses {
                tab_id,
                tab_generation,
                connection,
            } => self.load_dashboard_processes(tab_id, tab_generation, connection),
            Command::CancelDashboardTasks {
                tab_id,
                tab_generation,
            } => {
                let key = (tab_id, tab_generation);
                if let Some(task) = self.dashboard_metric_tasks.remove(&key) {
                    task.abort();
                }
                if let Some(task) = self.dashboard_metadata_tasks.remove(&key) {
                    task.abort();
                }
                if let Some(task) = self.dashboard_process_tasks.remove(&key) {
                    task.abort();
                }
            }
            Command::RunQueryPage {
                connection,
                target,
                tab_id,
                generation,
                source_sql,
                dialect,
                page,
            } => self.run_query_page(
                connection, target, tab_id, generation, source_sql, dialect, page,
            ),
            Command::RunDerivedQuery {
                connection,
                target,
                tab_id,
                source_generation,
                derived_generation,
                sql,
            } => self.run_derived_query(
                connection,
                target,
                tab_id,
                source_generation,
                derived_generation,
                sql,
            ),
            Command::RunDerivedQueryPage {
                connection,
                target,
                tab_id,
                source_generation,
                derived_generation,
                source_sql,
                where_clause,
                order_by_clause,
                dialect,
                page,
            } => self.run_derived_query_page(
                connection,
                target,
                tab_id,
                source_generation,
                derived_generation,
                source_sql,
                where_clause,
                order_by_clause,
                dialect,
                page,
            ),
            Command::ManualBegin {
                connection,
                target,
                tab_id,
                query_generation,
                transaction_generation,
            } => self.manual_begin(
                connection,
                target,
                tab_id,
                query_generation,
                transaction_generation,
            ),
            Command::ManualExecute {
                connection,
                target,
                tab_id,
                query_generation,
                transaction_generation,
                sql,
            } => self.manual_execute(
                connection,
                target,
                tab_id,
                query_generation,
                transaction_generation,
                sql,
            ),
            Command::ManualExecutePage {
                connection,
                target,
                tab_id,
                query_generation,
                transaction_generation,
                source_sql,
                dialect,
                count_sql,
                page,
            } => self.manual_execute_page(
                connection,
                target,
                tab_id,
                query_generation,
                transaction_generation,
                source_sql,
                dialect,
                count_sql,
                page,
            ),
            Command::ManualCommit {
                connection,
                tab_id,
                query_generation,
                transaction_generation,
            } => self.manual_commit(connection, tab_id, query_generation, transaction_generation),
            Command::ManualRollback {
                connection,
                tab_id,
                query_generation,
                transaction_generation,
            } => self.manual_rollback(connection, tab_id, query_generation, transaction_generation),
            Command::RelationMutation { request } => self.relation_mutation(request),
            Command::RelationCommit {
                tab_id,
                generation,
                connection,
            } => self.relation_transaction_end(tab_id, generation, connection, true),
            Command::RelationRollback {
                tab_id,
                generation,
                connection,
            } => self.relation_transaction_end(tab_id, generation, connection, false),
            Command::CancelQuery { tab_id, generation } => {
                if let Some(task) = self.query_tasks.remove(&(tab_id, generation)) {
                    task.abort();
                }
                if let Some(execution_id) = self.history_executions.remove(&(tab_id, generation))
                    && let Some(recorder) = self.history_recorder.clone()
                {
                    tokio::spawn(async move {
                        let _ = recorder
                            .finish_with_certainty(
                                execution_id,
                                crate::model::sql_history::HistoryExecutionStatus::Cancelled,
                                crate::model::sql_history::HistoryResultCertainty::Unknown,
                                None,
                                None,
                                None,
                            )
                            .await;
                    });
                }
            }
            Command::CancelManual {
                tab_id,
                transaction_generation,
                ..
            } => {
                if let Some(entry) = self.manual_transactions.get_mut(&tab_id)
                    && entry.transaction_generation == transaction_generation
                    && let Some(cancel) = entry.cancellation_sender.take()
                {
                    let _ = cancel.send(());
                }
            }
            Command::ScheduleCompletion(key) => {
                if let Some(task) = self.completion_tasks.remove(&key.console_id) {
                    task.abort();
                }
                let sender = self.event_sender.clone();
                let task = tokio::spawn(async move {
                    sleep(Duration::from_millis(120)).await;
                    let _ = sender.send(Action::CompletionDue(key));
                });
                self.completion_tasks.insert(key.console_id, task);
            }
            Command::ScheduleDiagnostics(key) => {
                let console_id = key.console_id;
                if let Some(task) = self.diagnostic_tasks.remove(&key.console_id) {
                    task.abort();
                }
                let sender = self.event_sender.clone();
                let task = tokio::spawn(async move {
                    sleep(Duration::from_millis(300)).await;
                    let _ = sender.send(Action::DiagnosticDue(key));
                });
                self.diagnostic_tasks.insert(console_id, task);
            }
            Command::AnalyzeDiagnostics {
                key,
                text,
                context,
                catalog,
            } => {
                let sender = self.event_sender.clone();
                tokio::spawn(async move {
                    let diagnostics = tokio::task::spawn_blocking(move || {
                        let syntax = crate::sql::diagnose_sql(&text, context.dialect);
                        if !syntax.is_empty() {
                            return (syntax, Vec::new());
                        }
                        let analysis = crate::sql::analyze_semantics(&text, &context, &catalog);
                        (analysis.diagnostics, analysis.dependencies)
                    })
                    .await
                    .unwrap_or_default();
                    let _ = sender.send(Action::DiagnosticsReady {
                        key,
                        diagnostics: diagnostics.0,
                        dependencies: diagnostics.1,
                    });
                });
            }
            Command::CheckForUpdate {
                request_id,
                automatic,
            } => {
                if self
                    .update_check_task
                    .as_ref()
                    .is_some_and(|task| !task.is_finished())
                {
                    return;
                }
                let sender = self.event_sender.clone();
                self.update_check_task = Some(tokio::spawn(async move {
                    match crate::update::inspect_current_installation(None, false).await {
                        Ok(inspection) => {
                            let _ = sender.send(Action::UpdateCheckCompleted {
                                request_id,
                                inspection,
                            });
                        }
                        Err(error) => {
                            let _ = sender.send(Action::UpdateCheckFailed {
                                request_id,
                                automatic,
                                message: error.to_string(),
                            });
                        }
                    }
                }));
            }
            Command::InstallUpdate {
                request_id,
                channel,
            } => {
                if self
                    .update_install_task
                    .as_ref()
                    .is_some_and(|task| !task.is_finished())
                {
                    return;
                }
                let sender = self.event_sender.clone();
                self.update_install_task = Some(tokio::spawn(async move {
                    match crate::update::install_current_native(Some(channel), false).await {
                        Ok(inspection) => {
                            let _ = sender.send(Action::UpdateInstalled {
                                request_id,
                                inspection,
                            });
                        }
                        Err(error) => {
                            let _ = sender.send(Action::UpdateInstallFailed {
                                request_id,
                                message: error.to_string(),
                            });
                        }
                    }
                }));
            }
            Command::ScheduleUpdateCheck { delay_ms } => {
                if self
                    .update_startup_task
                    .as_ref()
                    .is_some_and(|task| !task.is_finished())
                {
                    return;
                }
                let sender = self.event_sender.clone();
                self.update_startup_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    let _ = sender.send(Action::StartUpdateCheck { automatic: true });
                }));
            }
            Command::PersistWorkspace { revision, snapshot } => {
                if let Some(queue) = self.workspace_save.as_mut() {
                    queue.offer(revision, snapshot);
                    queue.poll(self.event_sender.clone());
                }
            }
            Command::FlushWorkspace { revision } => {
                if let Some(queue) = self.workspace_save.as_mut() {
                    queue.flush(revision, self.event_sender.clone());
                }
            }
            Command::ContinueWorkspaceSave => {
                if let Some(queue) = self.workspace_save.as_mut() {
                    queue.poll(self.event_sender.clone());
                }
            }
            Command::CompleteWorkspaceSave {
                revision,
                succeeded,
            } => {
                if let Some(queue) = self.workspace_save.as_mut() {
                    queue.complete(revision, succeeded, self.event_sender.clone());
                }
            }
            Command::RetryWorkspaceSave => {
                if let Some(queue) = self.workspace_save.as_mut() {
                    queue.retry(self.event_sender.clone());
                }
            }
            Command::DeleteSqlFile(id) => {
                if let Some(store) = self.workspace_store.clone() {
                    let mutation = Arc::clone(&self.workspace_mutation);
                    self.background_tasks.push(tokio::spawn(async move {
                        let _guard = mutation.lock().await;
                        let _ =
                            tokio::task::spawn_blocking(move || store.delete_sql_file(id)).await;
                    }));
                }
            }
            Command::WriteClipboard(payload) => {
                match self.clipboard.backend {
                    crate::config::ClipboardBackend::Off => {
                        let _ = self.event_sender.send(Action::ClipboardWriteFailed {
                            message: "Clipboard is disabled by configuration".to_owned(),
                        });
                        return;
                    }
                    crate::config::ClipboardBackend::Osc52 => {
                        let _ = self.event_sender.send(Action::Osc52Clipboard { payload });
                        return;
                    }
                    crate::config::ClipboardBackend::System => {}
                }
                let sender = self.event_sender.clone();
                task::spawn_blocking(move || {
                    let description = payload.description.clone();
                    let result = arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(payload.text));
                    let action = match result {
                        Ok(()) => Action::ClipboardWritten { description },
                        Err(_) => Action::ClipboardWriteFailed {
                            message: clipboard_write_failure_message(),
                        },
                    };
                    let _ = sender.send(action);
                });
            }
            Command::Quit => {}
        }
    }

    fn test_profile(&mut self, request_id: u64, submission: ProfileSubmission) {
        let registry = Arc::clone(&self.registry);
        let secret_store = Arc::clone(&self.secret_store);
        let local_credential_store = self.local_credential_store.clone();
        let sender = self.event_sender.clone();
        self.background_tasks.push(tokio::spawn(async move {
            let ProfileSubmission {
                profile,
                credential,
                discovery_fingerprint,
            } = submission;
            let password = match resolve_submission_password(
                &registry,
                &secret_store,
                &local_credential_store,
                &profile,
                credential,
            )
            .await
            {
                Ok(password) => password,
                Err(message) => {
                    let _ = sender.send(Action::ProfileTestFailed {
                        request_id,
                        message,
                    });
                    return;
                }
            };
            match DatabaseConnection::connect(&profile, password.as_ref()).await {
                Ok(database) => match database.probe().await {
                    Ok(server) => {
                        let capabilities = database.catalog_capabilities();
                        let discovery = database
                            .discover_catalog_scope()
                            .await
                            .map_err(|error| sanitize_terminal_text(&error.to_string()));
                        database.close().await;
                        let _ = sender.send(Action::ProfileTestSucceeded {
                            request_id,
                            fingerprint: discovery_fingerprint,
                            server,
                            capabilities,
                            discovery,
                        });
                    }
                    Err(error) => {
                        database.close().await;
                        let _ = sender.send(Action::ProfileTestFailed {
                            request_id,
                            message: sanitize_terminal_text(&error.to_string()),
                        });
                    }
                },
                Err(error) => {
                    let _ = sender.send(Action::ProfileTestFailed {
                        request_id,
                        message: sanitize_terminal_text(&error.to_string()),
                    });
                }
            }
        }));
    }

    fn check_secret_store_availability(&mut self) {
        let secret_store = Arc::clone(&self.secret_store);
        let sender = self.event_sender.clone();
        self.background_tasks.push(tokio::spawn(async move {
            let availability = SecretStoreAvailability::from_result(secret_store.available().await);
            let _ = sender.send(Action::SystemCredentialAvailability(availability));
        }));
    }

    fn discover_profile_catalog(&mut self, request_id: u64, submission: ProfileSubmission) {
        let registry = Arc::clone(&self.registry);
        let secret_store = Arc::clone(&self.secret_store);
        let local_credential_store = self.local_credential_store.clone();
        let sender = self.event_sender.clone();
        self.background_tasks.push(tokio::spawn(async move {
            let ProfileSubmission {
                profile,
                credential,
                discovery_fingerprint,
            } = submission;
            let password = match resolve_submission_password(
                &registry,
                &secret_store,
                &local_credential_store,
                &profile,
                credential,
            )
            .await
            {
                Ok(password) => password,
                Err(message) => {
                    let _ = sender.send(Action::ProfileCatalogDiscoveryFailed {
                        request_id,
                        fingerprint: discovery_fingerprint,
                        message,
                    });
                    return;
                }
            };
            match discover_profile_catalog(&profile, password.as_ref()).await {
                Ok((server, capabilities, discovery)) => {
                    let _ = sender.send(Action::ProfileCatalogDiscoverySucceeded {
                        request_id,
                        fingerprint: discovery_fingerprint,
                        server,
                        capabilities,
                        discovery,
                    });
                }
                Err(message) => {
                    let _ = sender.send(Action::ProfileCatalogDiscoveryFailed {
                        request_id,
                        fingerprint: discovery_fingerprint,
                        message,
                    });
                }
            }
        }));
    }

    fn save_profile(&mut self, request_id: u64, submission: ProfileSubmission, connect: bool) {
        let registry = Arc::clone(&self.registry);
        let mutation = Arc::clone(&self.profile_mutation);
        let profile_store = self.profile_store.clone();
        let secret_store = Arc::clone(&self.secret_store);
        let local_credential_store = self.local_credential_store.clone();
        let sender = self.event_sender.clone();
        self.profile_tasks.push(tokio::spawn(async move {
            let _mutation_guard = mutation.lock().await;
            match save_profile_transaction(
                registry,
                profile_store,
                secret_store,
                local_credential_store,
                submission,
            )
            .await
            {
                Ok(saved) => {
                    let _ = sender.send(Action::ProfileSaved {
                        request_id,
                        profile: saved.profile,
                        warning: saved.warning,
                        change: saved.change,
                        connect,
                    });
                }
                Err(message) => {
                    let _ = sender.send(Action::ProfileSaveFailed {
                        request_id,
                        message,
                    });
                }
            }
        }));
    }

    fn delete_profile(&mut self, request_id: u64, profile_id: Uuid) {
        let registry = Arc::clone(&self.registry);
        let mutation = Arc::clone(&self.profile_mutation);
        let profile_store = self.profile_store.clone();
        let secret_store = Arc::clone(&self.secret_store);
        let connection = Arc::clone(&self.connection);
        let sender = self.event_sender.clone();
        self.profile_tasks.push(tokio::spawn(async move {
            let _mutation_guard = mutation.lock().await;
            match delete_profile_transaction(
                registry,
                profile_store,
                secret_store,
                connection,
                profile_id,
            )
            .await
            {
                Ok(active_connection) => {
                    let _ = sender.send(Action::ProfileDeleted {
                        request_id,
                        profile_id,
                        active_connection,
                    });
                }
                Err(message) => {
                    let _ = sender.send(Action::ProfileDeleteFailed {
                        request_id,
                        message,
                    });
                }
            }
        }));
    }

    fn update_profile_access(
        &mut self,
        request_id: u64,
        profile_id: Uuid,
        change: ProfileAccessChange,
    ) {
        let registry = Arc::clone(&self.registry);
        let mutation = Arc::clone(&self.profile_mutation);
        let profile_store = self.profile_store.clone();
        let sender = self.event_sender.clone();
        self.profile_tasks.push(tokio::spawn(async move {
            let _mutation_guard = mutation.lock().await;
            let snapshot = registry.lock().await.clone();
            if !snapshot.profiles.contains_key(&profile_id) {
                let _ = sender.send(Action::ProfileAccessUpdateFailed {
                    request_id,
                    profile_id,
                    message: "Connection profile no longer exists".to_owned(),
                });
                return;
            };
            if !snapshot.persisted.contains(&profile_id) {
                let _ = sender.send(Action::ProfileAccessUpdateFailed {
                    request_id,
                    profile_id,
                    message: "Session connections have no saved access scope".to_owned(),
                });
                return;
            }
            let mut next = snapshot;
            let profile = next
                .profiles
                .get_mut(&profile_id)
                .expect("profile checked above");
            match change {
                ProfileAccessChange::MakeGlobal => {
                    profile.access = crate::profile::ProfileAccess::Global
                }
                ProfileAccessChange::MakeProjectOnly(root) => {
                    profile.access = crate::profile::ProfileAccess::Projects { roots: vec![root] };
                }
                ProfileAccessChange::AddProject(root) => {
                    if let crate::profile::ProfileAccess::Global = profile.access {
                        profile.access =
                            crate::profile::ProfileAccess::Projects { roots: Vec::new() };
                    }
                    profile.access.add_project(root);
                }
                ProfileAccessChange::RemoveProject(root) => profile.access.remove_project(&root),
            }
            let access = profile.access.clone();
            let collection = next.ordered_persisted_collection();
            if let Err(message) = save_profiles(profile_store, collection).await {
                let _ = sender.send(Action::ProfileAccessUpdateFailed {
                    request_id,
                    profile_id,
                    message,
                });
                return;
            }
            *registry.lock().await = next;
            let _ = sender.send(Action::ProfileAccessUpdated {
                request_id,
                profile_id,
                access,
            });
        }));
    }

    fn update_profile_organization(
        &mut self,
        request_id: u64,
        mutation: ProfileOrganizationMutation,
    ) {
        let registry = Arc::clone(&self.registry);
        let profile_mutation = Arc::clone(&self.profile_mutation);
        let store = self.profile_store.clone();
        let sender = self.event_sender.clone();
        self.profile_tasks.push(tokio::spawn(async move {
            let _guard = profile_mutation.lock().await;
            let snapshot = registry.lock().await.clone();
            let mut collection = snapshot.ordered_collection();
            let result = match mutation {
                ProfileOrganizationMutation::CreateGroup { id, name } => {
                    crate::model::profile_organization::create_group(&mut collection, id, name)
                        .map(|_| ())
                }
                ProfileOrganizationMutation::RenameGroup { group_id, name } => {
                    crate::model::profile_organization::rename_group(
                        &mut collection,
                        group_id,
                        name,
                    )
                }
                ProfileOrganizationMutation::DeleteGroup { group_id } => {
                    crate::model::profile_organization::delete_group(&mut collection, group_id)
                        .map(|_| ())
                }
                ProfileOrganizationMutation::AssignProfile {
                    profile_id,
                    group_id,
                } => crate::model::profile_organization::assign_profile(
                    &mut collection,
                    profile_id,
                    group_id,
                ),
                ProfileOrganizationMutation::MoveProfile {
                    profile_id,
                    sibling_ids,
                    direction,
                } => crate::model::profile_organization::move_profile(
                    &mut collection,
                    profile_id,
                    &sibling_ids,
                    direction,
                )
                .map(|_| ()),
            };
            if let Err(error) = result {
                let _ = sender.send(Action::ProfileOrganizationSaveFailed {
                    request_id,
                    message: sanitize_terminal_text(&error.to_string()),
                });
                return;
            }
            let persisted = ProfileCollection {
                groups: collection.groups.clone(),
                profiles: collection
                    .profiles
                    .iter()
                    .filter(|profile| snapshot.persisted.contains(&profile.id))
                    .cloned()
                    .collect(),
            };
            if let Err(error) = save_profiles(store, persisted).await {
                let _ = sender.send(Action::ProfileOrganizationSaveFailed {
                    request_id,
                    message: error,
                });
                return;
            }
            let mut next = snapshot;
            next.groups = collection.groups.clone();
            next.order = collection
                .profiles
                .iter()
                .map(|profile| profile.id)
                .collect();
            next.profiles = collection
                .profiles
                .iter()
                .cloned()
                .map(|profile| (profile.id, profile))
                .collect();
            *registry.lock().await = next;
            let _ = sender.send(Action::ProfileOrganizationSaved {
                request_id,
                collection,
            });
        }));
    }

    fn disconnect(&mut self, expected: ConnectionIdentity) {
        self.catalog_search_tasks
            .retain(|(owner, _), (identity, _, task)| {
                let should_abort =
                    *owner == crate::action::CatalogSearchOwner::Explorer && *identity == expected;
                if should_abort {
                    task.abort();
                    false
                } else {
                    true
                }
            });
        let connection = Arc::clone(&self.connection);
        let known_relations = Arc::clone(&self.known_relations);
        let known_relation_targets = Arc::clone(&self.known_relation_targets);
        let latest_catalog_requests = Arc::clone(&self.latest_catalog_requests);
        let mutation = Arc::clone(&self.profile_mutation);
        let attempts = Arc::clone(&self.connection_attempts);
        let sender = self.event_sender.clone();
        {
            let mut attempts = attempts.lock().expect("connection attempt mutex poisoned");
            attempts.cancel_matching(expected);
        }
        let manual_transactions = self
            .manual_transactions
            .keys()
            .copied()
            .filter(|tab_id| {
                self.manual_transactions
                    .get(tab_id)
                    .is_some_and(|entry| entry.connection == expected)
            })
            .collect::<Vec<_>>();
        let mut manual_entries = Vec::new();
        for tab_id in manual_transactions {
            if let Some(entry) = self.manual_transactions.remove(&tab_id) {
                manual_entries.push(entry);
            }
        }
        let relation_transactions = self
            .relation_transactions
            .keys()
            .copied()
            .filter(|tab_id| {
                self.relation_transactions
                    .get(tab_id)
                    .is_some_and(|entry| entry.connection == expected)
            })
            .collect::<Vec<_>>();
        let mut relation_entries = Vec::new();
        for tab_id in relation_transactions {
            if let Some(entry) = self.relation_transactions.remove(&tab_id) {
                relation_entries.push(entry);
            }
        }
        self.background_tasks.push(tokio::spawn(async move {
            for entry in manual_entries {
                shutdown_transaction_entry(entry).await;
            }
            for entry in relation_entries {
                let _ = entry
                    .request_sender
                    .send(crate::db::transaction::TransactionRequest::Shutdown);
                let _ = entry.worker_handle.await;
            }
            let _mutation_guard = mutation.lock().await;
            let active = {
                let mut guard = connection.lock().await;
                let keys = guard
                    .keys()
                    .filter(|key| key.identity == expected)
                    .cloned()
                    .collect::<Vec<_>>();
                keys.into_iter()
                    .filter_map(|key| guard.remove(&key))
                    .collect::<Vec<_>>()
            };
            if !active.is_empty() {
                if let Ok(mut known) = known_relations.lock() {
                    known.retain(|(identity, _)| *identity != expected);
                }
                if let Ok(mut targets) = known_relation_targets.lock() {
                    targets.retain(|(identity, _), _| *identity != expected);
                }
                if let Ok(mut latest) = latest_catalog_requests.lock() {
                    latest.retain(|(connection, _), _| *connection != expected);
                }
                for active in active {
                    active.database.close().await;
                }
            }
            let _ = sender.send(Action::DisconnectCompleted {
                connection: expected,
            });
        }));
    }

    fn connect(&mut self, profile_id: Uuid, generation: u64, target: ExecutionTarget) {
        let registry = Arc::clone(&self.registry);
        let mutation = Arc::clone(&self.profile_mutation);
        let secret_store = Arc::clone(&self.secret_store);
        let local_credential_store = self.local_credential_store.clone();
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let attempts = Arc::clone(&self.connection_attempts);
        let known_relations = Arc::clone(&self.known_relations);
        let known_relation_targets = Arc::clone(&self.known_relation_targets);
        let latest_catalog_requests = Arc::clone(&self.latest_catalog_requests);
        let expected = ConnectionIdentity {
            profile_id,
            generation,
        };
        let key = ConnectionKey::new(expected, target.clone());
        {
            let mut attempts = attempts.lock().expect("connection attempt mutex poisoned");
            if !attempts.start(key.clone()) {
                return;
            }
        }
        self.background_tasks.push(tokio::spawn(async move {
            let finish_attempt = || {
                attempts
                    .lock()
                    .expect("connection attempt mutex poisoned")
                    .finish(&key);
            };
            let mutation_guard = mutation.lock().await;
            let profile = {
                let registry = registry.lock().await;
                registry.profiles.get(&profile_id).cloned().map(|profile| {
                    let revision = registry.revisions.get(&profile_id).copied().unwrap_or(0);
                    (profile, revision)
                })
            };
            let Some((profile, profile_revision)) = profile else {
                finish_attempt();
                let _ = sender.send(Action::ConnectionFailed {
                    profile_id,
                    generation,
                    message: "Connection profile no longer exists".to_owned(),
                });
                return;
            };
            let password = match resolve_profile_password(
                &registry,
                &secret_store,
                &local_credential_store,
                &profile,
            )
            .await
            {
                Ok(password) => password,
                Err(message) => {
                    finish_attempt();
                    let _ = sender.send(Action::CredentialsRequired {
                        profile_id,
                        generation,
                        message,
                    });
                    return;
                }
            };
            drop(mutation_guard);
            if !target.is_valid(&profile) {
                finish_attempt();
                let _ = sender.send(Action::ConnectionFailed {
                    profile_id,
                    generation,
                    message: "Execution target is invalid for this profile".to_owned(),
                });
                return;
            }
            let reusable_sqlite = if profile.kind == crate::profile::DatabaseKind::Sqlite {
                connection
                    .lock()
                    .await
                    .get(&key)
                    .map(|active| active.database.clone())
            } else {
                None
            };
            let reused_sqlite = reusable_sqlite.is_some();
            let candidate = timeout(Duration::from_secs(10), async {
                let database = match reusable_sqlite {
                    Some(database) => database,
                    None => {
                        DatabaseConnection::connect_target(&profile, password.as_ref(), &target)
                            .await?
                    }
                };
                let server = match database.probe().await {
                    Ok(server) => server,
                    Err(error) => {
                        if !reused_sqlite {
                            database.close().await;
                        }
                        return Err(error);
                    }
                };
                let mutation_capabilities = database.catalog_mutation_capabilities();
                Ok::<_, crate::db::DatabaseError>((database, server, mutation_capabilities))
            })
            .await
            .map_err(|_| DatabaseError::configuration("connection timed out after 10 seconds"));
            match candidate {
                Ok(Ok((database, server, mutation_capabilities))) => {
                    let mutation_guard = mutation.lock().await;
                    if !profile_revision_is_current(&registry, &profile, profile_revision).await {
                        database.close().await;
                        finish_attempt();
                        return;
                    }
                    let mut active = connection.lock().await;
                    let mut candidate = Some(database);
                    let installation = {
                        let attempt_guard =
                            attempts.lock().expect("connection attempt mutex poisoned");
                        if !attempt_guard.is_current(&key) {
                            None
                        } else {
                            let replaced = active
                                .keys()
                                .find(|active_key| active_key.target == target)
                                .cloned()
                                .and_then(|active_key| {
                                    active
                                        .remove(&active_key)
                                        .map(|connection| (active_key, connection))
                                });
                            if let Some((replaced_key, _)) = &replaced {
                                if let Ok(mut known) = known_relations.lock() {
                                    known
                                        .retain(|(identity, _)| *identity != replaced_key.identity);
                                }
                                if let Ok(mut targets) = known_relation_targets.lock() {
                                    targets.retain(|(identity, _), _| {
                                        *identity != replaced_key.identity
                                    });
                                }
                                if let Ok(mut latest) = latest_catalog_requests.lock() {
                                    latest.retain(|(identity, _), _| {
                                        *identity != replaced_key.identity
                                    });
                                }
                            }
                            Some((
                                replaced,
                                active.insert(
                                    key.clone(),
                                    ActiveConnection {
                                        database: candidate
                                            .take()
                                            .expect("connection candidate exists"),
                                    },
                                ),
                            ))
                        }
                    };
                    drop(active);
                    let Some((replaced, previous)) = installation else {
                        let candidate = candidate.take().expect("connection candidate exists");
                        if !reused_sqlite {
                            candidate.close().await;
                        }
                        finish_attempt();
                        return;
                    };
                    let _ = sender.send(Action::ConnectionSucceeded {
                        profile_id,
                        generation,
                        server,
                        mutation_capabilities,
                    });
                    drop(mutation_guard);
                    if let Some((replaced_key, replaced)) = replaced
                        && replaced_key.identity != expected
                    {
                        replaced.database.close().await;
                    }
                    if let Some(previous) = previous
                        && !reused_sqlite
                    {
                        previous.database.close().await;
                    }
                }
                Ok(Err(error)) | Err(error) => {
                    let _mutation_guard = mutation.lock().await;
                    if profile_revision_is_current(&registry, &profile, profile_revision).await
                        && connection_attempt_is_current(&attempts, &key)
                    {
                        let _ = sender.send(Action::ConnectionFailed {
                            profile_id,
                            generation,
                            message: error.to_string(),
                        });
                    }
                }
            }
            finish_attempt();
        }));
    }

    fn load_catalog_page(&mut self, request: crate::db::catalog::CatalogRequest) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let known_relations = Arc::clone(&self.known_relations);
        let known_relation_targets = Arc::clone(&self.known_relation_targets);
        let latest_catalog_requests = Arc::clone(&self.latest_catalog_requests);
        if let Ok(mut latest) = latest_catalog_requests.lock() {
            latest.insert(
                (request.key.connection, request.key.target.clone()),
                request.key.clone(),
            );
        }
        self.background_tasks.push(tokio::spawn(async move {
            let key = request.key.clone();
            let database = {
                let guard = connection.lock().await;
                guard
                    .iter()
                    .find(|(connection_key, _)| connection_key.identity == key.connection)
                    .map(|(_, active)| active.database.clone())
            };
            let Some(database) = database else {
                let _ = sender.send(Action::CatalogPageFailed {
                    key,
                    category: crate::db::ErrorCategory::Internal,
                    message: "catalog request connection is no longer active".to_owned(),
                });
                return;
            };
            match database.load_catalog_page(&request).await {
                Ok(page) => {
                    if let Err(error) = page.validate_for(&request) {
                        let _ = sender.send(Action::CatalogPageFailed {
                            key,
                            category: crate::db::ErrorCategory::Internal,
                            message: format!("adapter returned invalid catalog page: {error}"),
                        });
                    } else {
                        let accepted = latest_catalog_requests.lock().is_ok_and(|latest| {
                            latest.get(&(request.key.connection, request.key.target.clone()))
                                == Some(&request.key)
                        });
                        let active =
                            active_database(Arc::clone(&connection), request.key.connection)
                                .await
                                .is_some();
                        if !active || !accepted {
                            return;
                        }
                        reconcile_known_relations(
                            &known_relations,
                            &known_relation_targets,
                            &request,
                            &page,
                        );
                        let _ = sender.send(Action::CatalogPageLoaded(page));
                    }
                }
                Err(error) => {
                    let accepted = latest_catalog_requests.lock().is_ok_and(|latest| {
                        latest.get(&(request.key.connection, request.key.target.clone()))
                            == Some(&request.key)
                    });
                    let active = active_database(Arc::clone(&connection), request.key.connection)
                        .await
                        .is_some();
                    if !active || !accepted {
                        return;
                    }
                    let _ = sender.send(Action::CatalogPageFailed {
                        key,
                        category: error.category,
                        message: error.to_string(),
                    });
                }
            }
        }));
    }

    fn resolve_catalog_relation(
        &mut self,
        connection: ConnectionIdentity,
        catalog_epoch: u64,
        request_id: u64,
        relation: crate::db::catalog::CatalogId,
        scope: crate::profile::CatalogScope,
    ) {
        let sender = self.event_sender.clone();
        let database = Arc::clone(&self.connection);
        self.background_tasks.push(tokio::spawn(async move {
            let Some(database) = active_database(Arc::clone(&database), connection).await else {
                let _ = sender.send(Action::CatalogRelationResolutionFailed {
                    connection,
                    catalog_epoch,
                    request_id,
                    relation,
                    category: crate::db::ErrorCategory::Internal,
                    message: "catalog relation request connection is no longer active".to_owned(),
                });
                return;
            };
            match database
                .resolve_relation_identity_with_scope(&relation, &scope)
                .await
            {
                Ok(entry) => {
                    let _ = sender.send(Action::CatalogRelationResolved {
                        connection,
                        catalog_epoch,
                        request_id,
                        relation,
                        entry,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogRelationResolutionFailed {
                        connection,
                        catalog_epoch,
                        request_id,
                        relation,
                        category: error.category,
                        message: error.to_string(),
                    });
                }
            }
        }));
    }

    fn reconcile_catalog_relation(
        &self,
        connection: ConnectionIdentity,
        old_relation: crate::db::catalog::CatalogId,
        new_relation: crate::db::catalog::CatalogId,
    ) {
        reconcile_catalog_relation(
            &self.known_relations,
            &self.known_relation_targets,
            connection,
            old_relation,
            new_relation,
        );
    }

    fn load_catalog_object_definition(
        &mut self,
        request: crate::db::catalog_mutation::CatalogObjectDefinitionRequest,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let registry = Arc::clone(&self.registry);
        let secret_store = Arc::clone(&self.secret_store);
        let local_credential_store = self.local_credential_store.clone();
        self.background_tasks.push(tokio::spawn(async move {
            let routed = match resolve_catalog_mutation_connection(
                Arc::clone(&connection),
                request.connection,
                crate::db::catalog_mutation::CatalogMutationTarget::Database(
                    request.target.clone(),
                ),
                &registry,
                &secret_store,
                &local_credential_store,
            )
            .await
            {
                Ok(routed) => routed,
                Err(message) => {
                    let _ =
                        sender.send(Action::CatalogObjectDefinitionLoadFailed { request, message });
                    return;
                }
            };
            match routed
                .database
                .load_catalog_object_definition(&request)
                .await
            {
                Ok(definition) => {
                    let _ = sender.send(Action::CatalogObjectDefinitionLoaded {
                        request,
                        definition,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogObjectDefinitionLoadFailed {
                        request,
                        message: sanitize_terminal_text(&error.to_string()),
                    });
                }
            }
            routed.close_if_owned().await;
        }));
    }

    fn load_catalog_owner_context(
        &mut self,
        request: crate::db::catalog_mutation::CatalogOwnerContextRequest,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let registry = Arc::clone(&self.registry);
        let secret_store = Arc::clone(&self.secret_store);
        let local_credential_store = self.local_credential_store.clone();
        self.background_tasks.push(tokio::spawn(async move {
            let routed = match resolve_catalog_mutation_connection(
                Arc::clone(&connection),
                request.connection,
                crate::db::catalog_mutation::CatalogMutationTarget::Database(
                    request.target.clone(),
                ),
                &registry,
                &secret_store,
                &local_credential_store,
            )
            .await
            {
                Ok(routed) => routed,
                Err(message) => {
                    let _ = sender.send(Action::CatalogOwnerContextLoadFailed { request, message });
                    return;
                }
            };
            match routed.database.load_catalog_owner_context(&request).await {
                Ok(Some(context)) => {
                    let _ = sender.send(Action::CatalogOwnerContextLoaded { request, context });
                }
                Ok(None) => {
                    let _ = sender.send(Action::CatalogOwnerContextLoadFailed {
                        request,
                        message: "owner role discovery is not supported for this connection".into(),
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogOwnerContextLoadFailed {
                        request,
                        message: sanitize_terminal_text(&error.to_string()),
                    });
                }
            }
            routed.close_if_owned().await;
        }));
    }

    fn plan_catalog_drop(&mut self, request: crate::db::catalog_drop::CatalogDropRequest) {
        let key = (request.connection, request.request_id);
        if self.catalog_drop_plan_tasks.contains_key(&key) {
            return;
        }
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let registry = Arc::clone(&self.registry);
        let task_request = request.clone();
        let task = tokio::spawn(async move {
            let fail = |error| {
                let _ = sender.send(Action::CatalogDropPlanFailed {
                    request: task_request.clone(),
                    error,
                });
            };
            if let Err(error) = task_request.validate() {
                fail(error);
                return;
            }
            let Some(database) =
                active_database(Arc::clone(&connection), task_request.connection).await
            else {
                fail(crate::db::catalog_drop::CatalogDropError::Unsupported {
                    kind: task_request.object.kind,
                    reason: "catalog drop connection is no longer active".to_owned(),
                });
                return;
            };
            let profile = registry
                .lock()
                .await
                .profiles
                .get(&task_request.connection.profile_id)
                .cloned();
            let Some(profile) = profile else {
                fail(crate::db::catalog_drop::CatalogDropError::Unsupported {
                    kind: task_request.object.kind,
                    reason: "catalog drop profile is no longer active".to_owned(),
                });
                return;
            };
            if profile.read_only {
                fail(crate::db::catalog_drop::CatalogDropError::Unsupported {
                    kind: task_request.object.kind,
                    reason: "catalog drop is unavailable on a read-only profile".to_owned(),
                });
                return;
            }
            let Some(entry) = task_request.entry.as_ref() else {
                fail(crate::db::catalog_drop::CatalogDropError::Unsupported {
                    kind: task_request.object.kind,
                    reason: "catalog drop request has no catalog entry".to_owned(),
                });
                return;
            };
            match database.plan_catalog_drop(task_request.clone(), entry) {
                Ok(plan) => {
                    let _ = sender.send(Action::CatalogDropPlanReady(plan));
                }
                Err(error) => fail(error),
            }
        });
        self.catalog_drop_plan_tasks.insert(key, task);
    }

    fn plan_catalog_mutation(
        &mut self,
        request: crate::db::catalog_mutation::CatalogMutationRequest,
        draft: crate::model::catalog_editor::CatalogDraft,
        baseline: Option<crate::db::catalog_mutation::CatalogObjectDefinition>,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task_request = request.clone();
        self.background_tasks.push(tokio::spawn(async move {
            let Some(database) = active_database(connection, request.connection).await else {
                let _ = sender.send(Action::CatalogMutationPlanFailed {
                    request: task_request,
                    message: "Active connection is no longer available".into(),
                });
                return;
            };
            match database.plan_catalog_mutation(request, draft, baseline) {
                Ok(plan) => {
                    let _ = sender.send(Action::CatalogMutationPlanReady(plan));
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogMutationPlanFailed {
                        request: task_request,
                        message: sanitize_terminal_text(&error.to_string()),
                    });
                }
            }
        }));
    }

    fn execute_catalog_mutation(&mut self, plan: crate::db::catalog_mutation::CatalogMutationPlan) {
        let key = (plan.request.connection, plan.request.request_id);
        if self.catalog_mutation_tasks.contains_key(&key) {
            return;
        }
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let registry = Arc::clone(&self.registry);
        let secret_store = Arc::clone(&self.secret_store);
        let local_credential_store = self.local_credential_store.clone();
        let task_plan = plan.clone();
        let task = tokio::spawn(async move {
            if let Err(error) = task_plan.validate() {
                let _ = sender.send(Action::CatalogMutationFailed {
                    plan: task_plan,
                    message: error.to_string(),
                });
                return;
            }
            let target = task_plan.execution_target.clone();
            let database = match resolve_catalog_mutation_connection(
                Arc::clone(&connection),
                task_plan.request.connection,
                target.clone(),
                &registry,
                &secret_store,
                &local_credential_store,
            )
            .await
            {
                Ok(database) => database,
                Err(message) => {
                    let _ = sender.send(Action::CatalogMutationFailed {
                        plan: task_plan,
                        message,
                    });
                    return;
                }
            };
            let Some(profile) = registry
                .lock()
                .await
                .profiles
                .get(&task_plan.request.connection.profile_id)
                .cloned()
            else {
                let _ = sender.send(Action::CatalogMutationFailed {
                    plan: task_plan,
                    message: "Mutation profile no longer exists".into(),
                });
                database.close_if_owned().await;
                return;
            };
            if profile.read_only {
                let _ = sender.send(Action::CatalogMutationFailed {
                    plan: task_plan,
                    message: "Schema mutation is unavailable on a read-only profile".into(),
                });
                database.close_if_owned().await;
                return;
            }
            if let Some(expected) = task_plan.baseline_fingerprint.as_deref()
                && let crate::db::catalog_mutation::CatalogMutationAnchor::Catalog(object) =
                    &task_plan.request.anchor
            {
                let definition_target =
                    target.execution_target(task_plan.request.connection.profile_id);
                let request = crate::db::catalog_mutation::CatalogObjectDefinitionRequest {
                    connection: task_plan.request.connection,
                    request_id: task_plan.request.request_id,
                    catalog_epoch: task_plan.request.catalog_epoch,
                    object: object.clone(),
                    target: definition_target,
                };
                match database
                    .database
                    .load_catalog_object_definition(&request)
                    .await
                {
                    Ok(definition)
                        if definition_baseline_fingerprint(&definition).as_deref()
                            == Some(expected) => {}
                    _ => {
                        let _ = sender.send(Action::CatalogMutationFailed {
                            plan: task_plan,
                            message: "Schema changed since the plan was created".into(),
                        });
                        database.close_if_owned().await;
                        return;
                    }
                }
            }
            match database.database.execute_catalog_mutation(&task_plan).await {
                Ok(outcome) => {
                    let _ = sender.send(Action::CatalogMutationSucceeded {
                        plan: task_plan,
                        outcome,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogMutationFailed {
                        plan: task_plan,
                        message: sanitize_terminal_text(&error.to_string()),
                    });
                }
            }
            database.close_if_owned().await;
        });
        self.catalog_mutation_tasks.insert(key, task);
    }

    fn execute_catalog_drop(&mut self, plan: crate::db::catalog_drop::CatalogDropPlan) {
        let key = (plan.request.connection, plan.request.request_id);
        if self.catalog_drop_execute_tasks.contains_key(&key) {
            return;
        }
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let registry = Arc::clone(&self.registry);
        let secret_store = Arc::clone(&self.secret_store);
        let local_credential_store = self.local_credential_store.clone();
        let target = plan.execution_target.clone();
        if let crate::db::catalog_drop::CatalogDropExecutionTarget::MaintenanceDatabase(
            maintenance_database,
        ) = target
        {
            let task_plan = plan.clone();
            let task = tokio::spawn(async move {
                execute_catalog_drop_on_maintenance(
                    task_plan,
                    maintenance_database,
                    connection,
                    registry,
                    secret_store,
                    local_credential_store,
                    sender,
                )
                .await;
            });
            self.catalog_drop_execute_tasks.insert(key, task);
            return;
        }
        let task_plan = plan.clone();
        let task = tokio::spawn(async move {
            if let Err(error) = task_plan.validate() {
                let _ = sender.send(Action::CatalogDropFailed {
                    plan: task_plan,
                    message: error.to_string(),
                });
                return;
            }
            let Some(database) =
                active_database(Arc::clone(&connection), task_plan.request.connection).await
            else {
                let _ = sender.send(Action::CatalogDropFailed {
                    plan: task_plan,
                    message: "catalog drop connection is no longer active".to_owned(),
                });
                return;
            };
            let profile = registry
                .lock()
                .await
                .profiles
                .get(&task_plan.request.connection.profile_id)
                .cloned();
            if profile.is_none() {
                let _ = sender.send(Action::CatalogDropFailed {
                    plan: task_plan,
                    message: "catalog drop profile is no longer active".to_owned(),
                });
                return;
            }
            if profile.is_some_and(|profile| profile.read_only) {
                let _ = sender.send(Action::CatalogDropFailed {
                    plan: task_plan,
                    message: "catalog drop is unavailable on a read-only profile".to_owned(),
                });
                return;
            }
            match database.execute(task_plan.sql()).await {
                Ok(outcome) => {
                    let _ = sender.send(Action::CatalogDropSucceeded {
                        plan: task_plan,
                        outcome,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogDropFailed {
                        plan: task_plan,
                        message: sanitize_terminal_text(&error.to_string()),
                    });
                }
            }
        });
        self.catalog_drop_execute_tasks.insert(key, task);
    }

    fn search_catalog(
        &mut self,
        owner: crate::action::CatalogSearchOwner,
        request: crate::db::catalog::CatalogSearchRequest,
    ) {
        let task_key = (owner, request.session_id);
        let task_generation = request.generation;
        if let Some((_, _, task)) = self.catalog_search_tasks.remove(&task_key) {
            task.abort();
        }
        let identity = request.connection;
        if let crate::action::CatalogSearchOwner::Explorer = owner {
            self.catalog_search_tasks
                .retain(|(existing_owner, _), (_, _, task)| {
                    if *existing_owner == owner {
                        task.abort();
                        false
                    } else {
                        true
                    }
                });
        }
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            sleep(Duration::from_millis(150)).await;
            let Some(database) = active_database(Arc::clone(&connection), request.connection).await
            else {
                return;
            };
            let identity = request.connection;
            let session_id = request.session_id;
            let generation = request.generation;
            match database.search_catalog(&request).await {
                Ok(page) => {
                    if active_database(connection, identity).await.is_some() {
                        let _ = sender.send(Action::CatalogSearchSucceeded { owner, page });
                    }
                }
                Err(error) => {
                    if active_database(connection, identity).await.is_some() {
                        let _ = sender.send(Action::CatalogSearchFailed {
                            owner,
                            connection: identity,
                            session_id,
                            generation,
                            message: sanitize_terminal_text(&error.to_string()),
                        });
                    }
                }
            }
        });
        self.catalog_search_tasks
            .insert(task_key, (identity, task_generation, task));
    }

    fn load_relation(&mut self, request: crate::model::relation::RelationRequest) {
        if request.relation.profile_id != request.connection.profile_id
            || !request.relation.object_id.kind.is_relation()
        {
            let _ = self.event_sender.send(Action::RelationFailed {
                request,
                message: "relation request is not owned by the active profile".to_owned(),
            });
            return;
        }
        if self.relation_tasks.contains_key(&request) {
            return;
        }
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task_request = request.clone();
        let task = tokio::spawn(async move {
            let Some(database) = active_database(connection, task_request.connection).await else {
                let _ = sender.send(Action::RelationFailed {
                    request: task_request,
                    message: "No active database connection".to_owned(),
                });
                return;
            };
            let result = match task_request.kind {
                crate::model::relation::RelationRequestKind::Preview => database
                    .preview_relation_with_scope(
                        &task_request.relation.object_id,
                        &task_request.scope,
                        &task_request.options,
                        task_request.page,
                    )
                    .await
                    .map(crate::model::relation::RelationSnapshot::Preview),
                crate::model::relation::RelationRequestKind::Ddl => database
                    .relation_ddl_with_scope(&task_request.relation.object_id, &task_request.scope)
                    .await
                    .map(|snapshot| {
                        crate::model::relation::RelationSnapshot::Ddl(Box::new(snapshot))
                    }),
            };
            match result {
                Ok(snapshot) => {
                    let _ = sender.send(Action::RelationSucceeded {
                        request: task_request,
                        snapshot: Box::new(snapshot),
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::RelationFailed {
                        request: task_request,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.relation_tasks.insert(request, task);
    }

    fn run_query(
        &mut self,
        expected: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        generation: u64,
        sql: String,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let recorder = self.history_recorder.clone();
        let execution_id = Uuid::new_v4();
        let operation_id = Uuid::new_v4();
        self.history_executions
            .insert((tab_id, generation), execution_id);
        let task = tokio::spawn(async move {
            let history = crate::model::sql_history::ExecutionHistory {
                execution_id,
                operation_id,
                transaction_id: None,
                sql: sql.clone(),
                status: crate::model::sql_history::HistoryExecutionStatus::Running,
                certainty: crate::model::sql_history::HistoryResultCertainty::Confirmed,
                transaction_outcome:
                    crate::model::sql_history::HistoryTransactionOutcome::NotApplicable,
                affected_rows: None,
                returned_rows: None,
                requested_at: chrono::Utc::now().timestamp_millis(),
                elapsed_millis: None,
                profile_id: Some(target.profile_id),
                database: Some(target.database.clone()),
                schema: target.schema.clone(),
            };
            if let Some(recorder) = &recorder {
                let _ = recorder.start(history).await;
            }
            let database = active_database_for_target(connection, expected, &target).await;
            let Some(database) = database else {
                if let Some(recorder) = &recorder {
                    let _ = recorder
                        .finish(
                            execution_id,
                            crate::model::sql_history::HistoryExecutionStatus::Failed,
                            None,
                            None,
                        )
                        .await;
                }
                let _ = sender.send(Action::QueryFailed {
                    tab_id,
                    generation,
                    connection: expected,
                    message: "Active connection does not match the execution target".to_owned(),
                });
                return;
            };
            match database
                .execute_with_budget(
                    &sql,
                    crate::db::query::QueryBudget {
                        max_rows: 10_000,
                        max_bytes: 16 * 1024 * 1024,
                    },
                )
                .await
            {
                Ok(outcome) => {
                    if let Some(recorder) = &recorder {
                        let _ = recorder
                            .finish_with_certainty(
                                execution_id,
                                crate::model::sql_history::HistoryExecutionStatus::Succeeded,
                                crate::model::sql_history::HistoryResultCertainty::Confirmed,
                                None,
                                Some(outcome.stats.row_count),
                                Some(outcome.stats.total().as_millis()),
                            )
                            .await;
                    }
                    let _ = sender.send(Action::QueryFinished {
                        tab_id,
                        generation,
                        connection: expected,
                        outcome,
                    });
                }
                Err(error) => {
                    if let Some(recorder) = &recorder {
                        let _ = recorder
                            .finish_with_certainty(
                                execution_id,
                                crate::model::sql_history::HistoryExecutionStatus::Failed,
                                crate::model::sql_history::HistoryResultCertainty::Confirmed,
                                None,
                                None,
                                None,
                            )
                            .await;
                    }
                    let _ = sender.send(Action::QueryFailed {
                        tab_id,
                        generation,
                        connection: expected,
                        message: error.output_message(),
                    });
                }
            }
        });
        self.query_tasks.insert((tab_id, generation), task);
    }

    fn load_sql_history(
        &mut self,
        generation: u64,
        request: crate::persistence::sql_history::HistoryPageRequest,
    ) {
        let Some(store) = self.history_store.clone() else {
            let _ = self.event_sender.send(Action::SqlHistoryLoadFailed {
                generation,
                message: "SQL history database is not available".into(),
            });
            return;
        };
        let sender = self.event_sender.clone();
        self.background_tasks.push(tokio::spawn(async move {
            match store.page(request).await {
                Ok(page) => {
                    let _ = sender.send(Action::SqlHistoryLoaded { generation, page });
                }
                Err(error) => {
                    let _ = sender.send(Action::SqlHistoryLoadFailed {
                        generation,
                        message: error.to_string(),
                    });
                }
            }
        }));
    }

    fn load_dashboard_metrics(
        &mut self,
        tab_id: Uuid,
        tab_generation: u64,
        expected: ConnectionIdentity,
    ) {
        let key = (tab_id, tab_generation);
        if self.dashboard_metric_tasks.contains_key(&key) {
            return;
        }
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database(connection, expected).await else {
                let _ = sender.send(Action::DashboardMetricsFailed {
                    tab_id,
                    tab_generation,
                    connection: expected,
                    message: "Active connection is no longer available".into(),
                });
                return;
            };
            match database.load_monitor_snapshot().await {
                Ok(snapshot) => {
                    let _ = sender.send(Action::DashboardMetricsLoaded {
                        tab_id,
                        tab_generation,
                        connection: expected,
                        snapshot,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::DashboardMetricsFailed {
                        tab_id,
                        tab_generation,
                        connection: expected,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.dashboard_metric_tasks.insert(key, task);
    }

    fn load_dashboard_metadata(
        &mut self,
        tab_id: Uuid,
        tab_generation: u64,
        expected: ConnectionIdentity,
    ) {
        let key = (tab_id, tab_generation);
        if self.dashboard_metadata_tasks.contains_key(&key) {
            return;
        }
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database(connection, expected).await else {
                return;
            };
            match database.load_monitor_metadata().await {
                Ok(metadata) => {
                    let _ = sender.send(Action::DashboardMetadataLoaded {
                        tab_id,
                        tab_generation,
                        connection: expected,
                        metadata,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::DashboardMetadataFailed {
                        tab_id,
                        tab_generation,
                        connection: expected,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.dashboard_metadata_tasks.insert(key, task);
    }

    fn load_dashboard_processes(
        &mut self,
        tab_id: Uuid,
        tab_generation: u64,
        expected: ConnectionIdentity,
    ) {
        let key = (tab_id, tab_generation);
        if self.dashboard_process_tasks.contains_key(&key) {
            return;
        }
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database(connection, expected).await else {
                let _ = sender.send(Action::DashboardProcessesFailed {
                    tab_id,
                    tab_generation,
                    connection: expected,
                    message: "Active connection is no longer available".into(),
                });
                return;
            };
            match database.load_process_snapshot().await {
                Ok(snapshot) => {
                    let _ = sender.send(Action::DashboardProcessesLoaded {
                        tab_id,
                        tab_generation,
                        connection: expected,
                        snapshot,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::DashboardProcessesFailed {
                        tab_id,
                        tab_generation,
                        connection: expected,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.dashboard_process_tasks.insert(key, task);
    }

    #[allow(clippy::too_many_arguments)]
    fn run_query_page(
        &mut self,
        expected: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        generation: u64,
        source_sql: String,
        dialect: crate::sql::SqlDialect,
        mut page: crate::model::pagination::PageRequest,
    ) {
        let operation_id = Uuid::new_v4();
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let recorder = self.history_recorder.clone();
        let task = tokio::spawn(async move {
            let Some(database) = active_database_for_target(connection, expected, &target).await
            else {
                let _ = sender.send(Action::QueryPageFailed {
                    tab_id,
                    generation,
                    connection: expected,
                    message: "Active connection does not match the execution target".into(),
                });
                return;
            };
            let query = match crate::sql::build_paginated_query(&source_sql, dialect, page) {
                Ok(query) => query,
                Err(error) => {
                    let _ = sender.send(Action::QueryPageFailed {
                        tab_id,
                        generation,
                        connection: expected,
                        message: error.to_string(),
                    });
                    return;
                }
            };
            let total = if page.resolve_total {
                let count_execution_id = Uuid::new_v4();
                if let Some(recorder) = &recorder {
                    let _ = recorder
                        .start(crate::model::sql_history::ExecutionHistory {
                            execution_id: count_execution_id,
                            operation_id,
                            transaction_id: None,
                            sql: query.count_sql.clone(),
                            status: crate::model::sql_history::HistoryExecutionStatus::Running,
                            certainty: crate::model::sql_history::HistoryResultCertainty::Confirmed,
                            transaction_outcome:
                                crate::model::sql_history::HistoryTransactionOutcome::NotApplicable,
                            affected_rows: None,
                            returned_rows: None,
                            requested_at: chrono::Utc::now().timestamp_millis(),
                            elapsed_millis: None,
                            profile_id: Some(target.profile_id),
                            database: Some(target.database.clone()),
                            schema: target.schema.clone(),
                        })
                        .await;
                }
                match database.execute(&query.count_sql).await {
                    Ok(outcome) => match count_from_outcome(&outcome) {
                        Ok(total) => {
                            if let Some(recorder) = &recorder {
                                let _ = recorder
                                    .finish_with_certainty(
                                        count_execution_id,
                                        crate::model::sql_history::HistoryExecutionStatus::Succeeded,
                                        crate::model::sql_history::HistoryResultCertainty::Confirmed,
                                        None,
                                        Some(outcome.stats.row_count),
                                        Some(outcome.stats.total().as_millis()),
                                    )
                                    .await;
                            }
                            Some(total)
                        }
                        Err(error) => {
                            if let Some(recorder) = &recorder {
                                let _ = recorder
                                    .finish_with_certainty(
                                        count_execution_id,
                                        crate::model::sql_history::HistoryExecutionStatus::Failed,
                                        crate::model::sql_history::HistoryResultCertainty::Confirmed,
                                        None,
                                        None,
                                        Some(outcome.stats.total().as_millis()),
                                    )
                                    .await;
                            }
                            let _ = sender.send(Action::QueryPageFailed {
                                tab_id,
                                generation,
                                connection: expected,
                                message: error.0,
                            });
                            return;
                        }
                    },
                    Err(error) => {
                        if let Some(recorder) = &recorder {
                            let _ = recorder
                                .finish_with_certainty(
                                    count_execution_id,
                                    crate::model::sql_history::HistoryExecutionStatus::Failed,
                                    crate::model::sql_history::HistoryResultCertainty::Confirmed,
                                    None,
                                    None,
                                    None,
                                )
                                .await;
                        }
                        let _ = sender.send(Action::QueryPageFailed {
                            tab_id,
                            generation,
                            connection: expected,
                            message: error.output_message(),
                        });
                        return;
                    }
                }
            } else {
                None
            };
            if let Some(total) = total {
                page.offset =
                    crate::model::pagination::ResultPagination::last_offset(page.size, total);
            }
            let query = match crate::sql::build_paginated_query(&source_sql, dialect, page) {
                Ok(query) => query,
                Err(error) => {
                    let _ = sender.send(Action::QueryPageFailed {
                        tab_id,
                        generation,
                        connection: expected,
                        message: error.to_string(),
                    });
                    return;
                }
            };
            let page_execution_id = Uuid::new_v4();
            if let Some(recorder) = &recorder {
                let _ = recorder
                    .start(crate::model::sql_history::ExecutionHistory {
                        execution_id: page_execution_id,
                        operation_id,
                        transaction_id: None,
                        sql: query.page_sql.clone(),
                        status: crate::model::sql_history::HistoryExecutionStatus::Running,
                        certainty: crate::model::sql_history::HistoryResultCertainty::Confirmed,
                        transaction_outcome:
                            crate::model::sql_history::HistoryTransactionOutcome::NotApplicable,
                        affected_rows: None,
                        returned_rows: None,
                        requested_at: chrono::Utc::now().timestamp_millis(),
                        elapsed_millis: None,
                        profile_id: Some(target.profile_id),
                        database: Some(target.database.clone()),
                        schema: target.schema.clone(),
                    })
                    .await;
            }
            match database.execute(&query.page_sql).await {
                Ok(mut outcome) => {
                    if let Some(recorder) = &recorder {
                        let _ = recorder
                            .finish_with_certainty(
                                page_execution_id,
                                crate::model::sql_history::HistoryExecutionStatus::Succeeded,
                                crate::model::sql_history::HistoryResultCertainty::Confirmed,
                                None,
                                Some(outcome.stats.row_count),
                                Some(outcome.stats.total().as_millis()),
                            )
                            .await;
                    }
                    let fetched = outcome.stats.row_count;
                    if let Some(result) = outcome.result_sets.first_mut() {
                        result.rows.truncate(page.size.get());
                    }
                    outcome.stats.row_count = outcome
                        .result_sets
                        .iter()
                        .map(|result| result.rows.len())
                        .sum();
                    let mut pagination =
                        crate::model::pagination::ResultPagination::from_page(page, fetched);
                    if let Some(total) = total {
                        pagination.total = crate::model::pagination::TotalRows::Exact(total);
                        pagination.has_next =
                            page.offset.saturating_add(pagination.visible_rows as u64) < total;
                    }
                    let _ = sender.send(Action::QueryPageFinished {
                        tab_id,
                        generation,
                        connection: expected,
                        outcome,
                        pagination,
                    });
                }
                Err(error) => {
                    if let Some(recorder) = &recorder {
                        let _ = recorder
                            .finish_with_certainty(
                                page_execution_id,
                                crate::model::sql_history::HistoryExecutionStatus::Failed,
                                crate::model::sql_history::HistoryResultCertainty::Confirmed,
                                None,
                                None,
                                None,
                            )
                            .await;
                    }
                    let _ = sender.send(Action::QueryPageFailed {
                        tab_id,
                        generation,
                        connection: expected,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.query_tasks.insert((tab_id, generation), task);
    }

    fn run_derived_query(
        &mut self,
        expected: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        source_generation: u64,
        derived_generation: u64,
        sql: String,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database_for_target(connection, expected, &target).await
            else {
                let _ = sender.send(Action::DerivedQueryFailed {
                    tab_id,
                    source_generation,
                    derived_generation,
                    connection: expected,
                    target,
                    message: "Active connection does not match the execution target".into(),
                });
                return;
            };
            match database
                .execute_with_budget(
                    &sql,
                    crate::db::query::QueryBudget {
                        max_rows: 10_000,
                        max_bytes: 16 * 1024 * 1024,
                    },
                )
                .await
            {
                Ok(outcome) => {
                    let _ = sender.send(Action::DerivedQueryFinished {
                        tab_id,
                        source_generation,
                        derived_generation,
                        connection: expected,
                        target,
                        outcome,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::DerivedQueryFailed {
                        tab_id,
                        source_generation,
                        derived_generation,
                        connection: expected,
                        target,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.query_tasks.insert((tab_id, derived_generation), task);
    }

    #[allow(clippy::too_many_arguments)]
    fn run_derived_query_page(
        &mut self,
        expected: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        source_generation: u64,
        derived_generation: u64,
        source_sql: String,
        where_clause: String,
        order_by_clause: String,
        dialect: crate::sql::SqlDialect,
        mut page: crate::model::pagination::PageRequest,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database_for_target(connection, expected, &target).await
            else {
                let _ = sender.send(Action::DerivedQueryPageFailed {
                    tab_id,
                    source_generation,
                    derived_generation,
                    connection: expected,
                    target,
                    message: "Active connection does not match the execution target".into(),
                });
                return;
            };
            let query = match crate::sql::build_derived_paginated_query(
                &source_sql,
                &where_clause,
                &order_by_clause,
                dialect,
                page,
            ) {
                Ok(query) => query,
                Err(error) => {
                    let _ = sender.send(Action::DerivedQueryPageFailed {
                        tab_id,
                        source_generation,
                        derived_generation,
                        connection: expected,
                        target,
                        message: error.to_string(),
                    });
                    return;
                }
            };
            let total = if page.resolve_total {
                match database.execute(&query.count_sql).await {
                    Ok(outcome) => match count_from_outcome(&outcome) {
                        Ok(total) => Some(total),
                        Err(error) => {
                            let _ = sender.send(Action::DerivedQueryPageFailed {
                                tab_id,
                                source_generation,
                                derived_generation,
                                connection: expected,
                                target,
                                message: error.0,
                            });
                            return;
                        }
                    },
                    Err(error) => {
                        let _ = sender.send(Action::DerivedQueryPageFailed {
                            tab_id,
                            source_generation,
                            derived_generation,
                            connection: expected,
                            target,
                            message: error.to_string(),
                        });
                        return;
                    }
                }
            } else {
                None
            };
            if let Some(total) = total {
                page.offset =
                    crate::model::pagination::ResultPagination::last_offset(page.size, total);
            }
            let query = match crate::sql::build_derived_paginated_query(
                &source_sql,
                &where_clause,
                &order_by_clause,
                dialect,
                page,
            ) {
                Ok(query) => query,
                Err(error) => {
                    let _ = sender.send(Action::DerivedQueryPageFailed {
                        tab_id,
                        source_generation,
                        derived_generation,
                        connection: expected,
                        target,
                        message: error.to_string(),
                    });
                    return;
                }
            };
            match database.execute(&query.page_sql).await {
                Ok(mut outcome) => {
                    let fetched = outcome.stats.row_count;
                    if let Some(result) = outcome.result_sets.first_mut() {
                        result.rows.truncate(page.size.get());
                    }
                    outcome.stats.row_count = outcome
                        .result_sets
                        .iter()
                        .map(|result| result.rows.len())
                        .sum();
                    let mut pagination =
                        crate::model::pagination::ResultPagination::from_page(page, fetched);
                    if let Some(total) = total {
                        pagination.total = crate::model::pagination::TotalRows::Exact(total);
                        pagination.has_next =
                            page.offset.saturating_add(pagination.visible_rows as u64) < total;
                    }
                    let _ = sender.send(Action::DerivedQueryPageFinished {
                        tab_id,
                        source_generation,
                        derived_generation,
                        connection: expected,
                        target,
                        outcome,
                        pagination,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::DerivedQueryPageFailed {
                        tab_id,
                        source_generation,
                        derived_generation,
                        connection: expected,
                        target,
                        message: error.to_string(),
                    });
                }
            }
        });
        self.query_tasks.insert((tab_id, derived_generation), task);
    }

    fn manual_begin(
        &mut self,
        connection: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
    ) {
        self.reap_finished_manual_worker(tab_id);
        self.ensure_manual_worker(
            connection,
            target,
            tab_id,
            query_generation,
            transaction_generation,
            None,
        );
    }

    fn relation_mutation(&mut self, request: crate::db::mutation::RelationMutationRequest) {
        let tab_id = request.tab_id;
        if self
            .relation_mutation_blocked
            .lock()
            .is_ok_and(|blocked| blocked.contains(&(tab_id, request.connection)))
        {
            let _ = self.event_sender.send(Action::RelationMutationFailed {
                request,
                message: "Relation transaction outcome is unknown; reconnect before mutating"
                    .into(),
                diagnostic: None,
            });
            return;
        }
        if self
            .relation_transactions
            .get(&tab_id)
            .is_some_and(|entry| entry.worker_handle.is_finished())
        {
            self.relation_transactions.remove(&tab_id);
        }
        let generation = request.edit_generation;
        let (reply, result) = tokio::sync::oneshot::channel();
        let (cancel, cancellation) = tokio::sync::oneshot::channel();
        self.ensure_relation_worker(request.clone(), cancellation, reply);
        let sender = self.event_sender.clone();
        let blocked = Arc::clone(&self.relation_mutation_blocked);
        self.background_tasks.push(tokio::spawn(async move {
            match result.await {
                Ok(Ok(result)) => {
                    let _ = sender.send(Action::RelationMutationSucceeded { request, result });
                }
                Ok(Err(error)) => {
                    let _ = sender.send(Action::RelationMutationFailed {
                        request,
                        message: error.relation_diagnostic().as_ref().map_or_else(
                            || error.0.clone(),
                            crate::db::transaction::RelationMutationDiagnostic::safe_message,
                        ),
                        diagnostic: error.relation_diagnostic(),
                    });
                }
                Err(_) => {
                    if let Ok(mut blocked) = blocked.lock() {
                        blocked.insert((tab_id, request.connection));
                    }
                    let _ = sender.send(Action::RelationMutationFailed {
                        request,
                        message: "Relation mutation acknowledgement was lost".into(),
                        diagnostic: None,
                    });
                }
            }
        }));
        if let Some(entry) = self.relation_transactions.get_mut(&tab_id) {
            entry.cancellation_sender = Some(cancel);
        }
        let _ = generation;
    }

    fn relation_transaction_end(
        &mut self,
        tab_id: Uuid,
        generation: u64,
        connection: ConnectionIdentity,
        commit: bool,
    ) {
        self.relation_transactions
            .retain(|_, entry| !entry.worker_handle.is_finished());
        let Some(entry) = self.relation_transactions.get(&tab_id) else {
            let action = if commit {
                Action::RelationCommitFailed {
                    tab_id,
                    generation,
                    connection,
                    message: "Relation transaction acknowledgement was lost".into(),
                    unknown: true,
                }
            } else {
                Action::RelationRollbackFailed {
                    tab_id,
                    generation,
                    connection,
                    message: "Relation transaction acknowledgement was lost".into(),
                    unknown: true,
                }
            };
            let _ = self.event_sender.send(action);
            self.relation_mutation_blocked
                .lock()
                .expect("relation mutation gate mutex poisoned")
                .insert((tab_id, connection));
            return;
        };
        let (reply, result) = tokio::sync::oneshot::channel();
        let request = if commit {
            TransactionRequest::Commit { reply }
        } else {
            TransactionRequest::Rollback { reply }
        };
        if entry.request_sender.send(request).is_err() {
            let message = "Relation transaction acknowledgement was lost".into();
            let action = if commit {
                Action::RelationCommitFailed {
                    tab_id,
                    generation,
                    connection,
                    message,
                    unknown: true,
                }
            } else {
                Action::RelationRollbackFailed {
                    tab_id,
                    generation,
                    connection,
                    message,
                    unknown: true,
                }
            };
            let _ = self.event_sender.send(action);
            self.relation_mutation_blocked
                .lock()
                .expect("relation mutation gate mutex poisoned")
                .insert((tab_id, connection));
            return;
        }
        let sender = self.event_sender.clone();
        let blocked = Arc::clone(&self.relation_mutation_blocked);
        self.background_tasks.push(tokio::spawn(async move {
            match result.await {
                Ok(Ok(())) => {
                    if let Ok(mut blocked) = blocked.lock() {
                        blocked.remove(&(tab_id, connection));
                    }
                    let _ = sender.send(if commit {
                        Action::RelationCommitted {
                            tab_id,
                            generation,
                            connection,
                        }
                    } else {
                        Action::RelationRolledBack {
                            tab_id,
                            generation,
                            connection,
                        }
                    });
                }
                Ok(Err(error)) => {
                    if let Ok(mut blocked) = blocked.lock() {
                        blocked.insert((tab_id, connection));
                    }
                    let message = error.0;
                    let _ = sender.send(if commit {
                        Action::RelationCommitFailed {
                            tab_id,
                            generation,
                            connection,
                            message,
                            unknown: true,
                        }
                    } else {
                        Action::RelationRollbackFailed {
                            tab_id,
                            generation,
                            connection,
                            message,
                            unknown: true,
                        }
                    });
                }
                Err(_) => {
                    if let Ok(mut blocked) = blocked.lock() {
                        blocked.insert((tab_id, connection));
                    }
                    let message = "Relation transaction acknowledgement was lost".into();
                    let _ = sender.send(if commit {
                        Action::RelationCommitFailed {
                            tab_id,
                            generation,
                            connection,
                            message,
                            unknown: true,
                        }
                    } else {
                        Action::RelationRollbackFailed {
                            tab_id,
                            generation,
                            connection,
                            message,
                            unknown: true,
                        }
                    });
                }
            }
        }));
    }

    fn ensure_relation_worker(
        &mut self,
        request: crate::db::mutation::RelationMutationRequest,
        cancel: tokio::sync::oneshot::Receiver<()>,
        reply: tokio::sync::oneshot::Sender<
            Result<crate::db::mutation::MutationResult, crate::db::transaction::TransactionError>,
        >,
    ) {
        let tab_id = request.tab_id;
        if let Some(entry) = self.relation_transactions.get(&tab_id) {
            let _ = entry
                .request_sender
                .send(TransactionRequest::RelationMutation {
                    request,
                    cancel,
                    reply,
                });
            return;
        }
        let (proxy, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let active = Arc::clone(&self.connection);
        let sender = self.event_sender.clone();
        let connection = request.connection;
        let transaction_generation = request.edit_generation;
        let target = request.target.clone();
        let worker_target = target.clone();
        let worker_request = request.clone();
        let forced_close = ForcedCloseHandle::new();
        let worker_forced_close = forced_close.clone();
        let worker_handle = tokio::spawn(async move {
            let Some(database) =
                active_database_for_target(active.clone(), connection, &worker_target).await
            else {
                let _ = sender.send(Action::RelationTransactionStartFailed {
                    tab_id,
                    generation: request.edit_generation,
                    connection,
                    message: "No active database connection".into(),
                });
                let _ = sender.send(Action::RelationMutationFailed {
                    request: worker_request.clone(),
                    message: "No active database connection".into(),
                    diagnostic: None,
                });
                return crate::db::transaction::WorkerDisposition::Quarantine;
            };
            let invalidates = matches!(database, DatabaseConnection::Sqlite(_));
            let worker = match database
                .start_transaction_worker_with_forced_close(worker_forced_close)
                .await
            {
                Ok(worker) => worker,
                Err(error) => {
                    let _ = sender.send(Action::RelationTransactionStartFailed {
                        tab_id,
                        generation: request.edit_generation,
                        connection,
                        message: error.to_string(),
                    });
                    let _ = sender.send(Action::RelationMutationFailed {
                        request: worker_request.clone(),
                        message: error.to_string(),
                        diagnostic: None,
                    });
                    return crate::db::transaction::WorkerDisposition::Quarantine;
                }
            };
            let crate::runtime::transaction::TransactionWorkerHandle {
                requests,
                worker,
                readiness,
                ..
            } = worker;
            if let Ok(Ok(())) = readiness.await {
                let _ = sender.send(Action::RelationTransactionStarted {
                    tab_id,
                    generation: request.edit_generation,
                    connection,
                });
            } else {
                let _ = sender.send(Action::RelationTransactionStartFailed {
                    tab_id,
                    generation: request.edit_generation,
                    connection,
                    message: "Relation transaction could not be started".into(),
                });
                let _ = sender.send(Action::RelationMutationFailed {
                    request: worker_request.clone(),
                    message: "Relation transaction could not be started".into(),
                    diagnostic: None,
                });
                return crate::db::transaction::WorkerDisposition::Quarantine;
            }
            tokio::pin!(worker);
            loop {
                tokio::select! {
                    item = receiver.recv() => { let Some(item) = item else { break }; if requests.send(item).is_err() { break; } }
                    disposition = &mut worker => { let disposition = disposition.unwrap_or(crate::db::transaction::WorkerDisposition::Quarantine); if disposition == crate::db::transaction::WorkerDisposition::Quarantine && invalidates { handle_quarantined_connection(active.clone(), sender.clone(), connection).await; } return disposition; }
                }
            }
            worker
                .await
                .unwrap_or(crate::db::transaction::WorkerDisposition::Quarantine)
        });
        self.relation_transactions.insert(
            tab_id,
            ManualTransactionEntry {
                connection,
                target,
                transaction_id: Uuid::new_v4(),
                transaction_generation,
                request_sender: proxy.clone(),
                worker_handle,
                cancellation_sender: None,
                forced_close_handle: forced_close,
            },
        );
        let _ = proxy.send(TransactionRequest::RelationMutation {
            request,
            cancel,
            reply,
        });
    }

    fn manual_execute(
        &mut self,
        connection: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
        sql: String,
    ) {
        self.reap_finished_manual_worker(tab_id);
        let (reply, result) = tokio::sync::oneshot::channel();
        let (cancel, cancel_receiver) = tokio::sync::oneshot::channel();
        let history_sql = sql.clone();
        let history_target = target.clone();
        self.ensure_manual_worker(
            connection,
            target,
            tab_id,
            query_generation,
            transaction_generation,
            None,
        );
        if let Some(entry) = self.manual_transactions.get_mut(&tab_id) {
            let execution_id = Uuid::new_v4();
            let operation_id = Uuid::new_v4();
            let transaction_id = entry.transaction_id;
            let recorder = self.history_recorder.clone();
            let history = crate::model::sql_history::ExecutionHistory {
                execution_id,
                operation_id,
                transaction_id: Some(transaction_id),
                sql: history_sql,
                status: crate::model::sql_history::HistoryExecutionStatus::Running,
                certainty: crate::model::sql_history::HistoryResultCertainty::Confirmed,
                transaction_outcome: crate::model::sql_history::HistoryTransactionOutcome::Pending,
                affected_rows: None,
                returned_rows: None,
                requested_at: chrono::Utc::now().timestamp_millis(),
                elapsed_millis: None,
                profile_id: Some(history_target.profile_id),
                database: Some(history_target.database.clone()),
                schema: history_target.schema.clone(),
            };
            if let Some(recorder) = &recorder {
                let recorder = recorder.clone();
                let request_sender = entry.request_sender.clone();
                let request_sql = sql;
                self.background_tasks.push(tokio::spawn(async move {
                    let _ = recorder.start(history).await;
                    let _ = request_sender.send(TransactionRequest::Execute {
                        query_generation,
                        sql: request_sql,
                        cancel: cancel_receiver,
                        reply,
                    });
                }));
            } else {
                let _ = entry.request_sender.send(TransactionRequest::Execute {
                    query_generation,
                    sql,
                    cancel: cancel_receiver,
                    reply,
                });
            }
            entry.cancellation_sender = Some(cancel);
            let sender = self.event_sender.clone();
            self.query_tasks.insert(
                (tab_id, query_generation),
                tokio::spawn(async move {
                    match result.await {
                        Ok(Ok(outcome)) => {
                            if let Some(recorder) = &recorder {
                                let _ = recorder
                                    .finish_with_certainty(
                                        execution_id,
                                        crate::model::sql_history::HistoryExecutionStatus::Succeeded,
                                        crate::model::sql_history::HistoryResultCertainty::Confirmed,
                                        None,
                                        Some(outcome.stats.row_count),
                                        Some(outcome.stats.total().as_millis()),
                                    )
                                    .await;
                            }
                            let _ = sender.send(Action::ManualQueryFinished {
                                tab_id,
                                query_generation,
                                transaction_generation,
                                connection,
                                outcome,
                            });
                        }
                        Ok(Err(error)) => {
                            if let Some(recorder) = &recorder {
                                let status = if error.0 == "cancelled" {
                                    crate::model::sql_history::HistoryExecutionStatus::Cancelled
                                } else {
                                    crate::model::sql_history::HistoryExecutionStatus::Failed
                                };
                                let certainty = if error.0 == "cancelled" {
                                    crate::model::sql_history::HistoryResultCertainty::Unknown
                                } else {
                                    crate::model::sql_history::HistoryResultCertainty::Confirmed
                                };
                                let _ = recorder
                                    .finish_with_certainty(
                                        execution_id,
                                        status,
                                        certainty,
                                        None,
                                        None,
                                        None,
                                    )
                                    .await;
                            }
                            let _ = sender.send(Action::ManualQueryFailed {
                                tab_id,
                                query_generation,
                                transaction_generation,
                                connection,
                                message: error.0,
                            });
                        }
                        Err(_) => {
                            if let Some(recorder) = &recorder {
                                let _ = recorder
                                    .finish_with_certainty(
                                        execution_id,
                                        crate::model::sql_history::HistoryExecutionStatus::Interrupted,
                                        crate::model::sql_history::HistoryResultCertainty::Unknown,
                                        None,
                                        None,
                                        None,
                                    )
                                    .await;
                            }
                            let _ = sender.send(Action::ManualQueryFailed {
                                tab_id,
                                query_generation,
                                transaction_generation,
                                connection,
                                message: "Manual query acknowledgement was lost".to_owned(),
                            });
                        }
                    }
                }),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn manual_execute_page(
        &mut self,
        connection: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
        source_sql: String,
        dialect: crate::sql::SqlDialect,
        count_sql: String,
        page: crate::model::pagination::PageRequest,
    ) {
        self.reap_finished_manual_worker(tab_id);
        let (reply, result) = tokio::sync::oneshot::channel();
        self.ensure_manual_worker_page(
            connection,
            target,
            tab_id,
            query_generation,
            transaction_generation,
            source_sql,
            dialect,
            count_sql,
            page,
            reply,
        );
        let sender = self.event_sender.clone();
        self.query_tasks.insert(
            (tab_id, query_generation),
            tokio::spawn(async move {
                match result.await {
                    Ok(Ok((outcome, pagination))) => {
                        let _ = sender.send(Action::ManualQueryPageFinished {
                            tab_id,
                            query_generation,
                            transaction_generation,
                            connection,
                            outcome,
                            pagination,
                        });
                    }
                    Ok(Err(error)) => {
                        let _ = sender.send(Action::ManualQueryPageFailed {
                            tab_id,
                            query_generation,
                            transaction_generation,
                            connection,
                            message: error.0,
                        });
                    }
                    Err(_) => {
                        let _ = sender.send(Action::ManualQueryPageFailed {
                            tab_id,
                            query_generation,
                            transaction_generation,
                            connection,
                            message: "Manual query acknowledgement was lost".into(),
                        });
                    }
                }
            }),
        );
    }

    fn manual_commit(
        &mut self,
        connection: ConnectionIdentity,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
    ) {
        self.reap_finished_manual_worker(tab_id);
        let Some(entry) = self.manual_transactions.get(&tab_id) else {
            return;
        };
        let (reply, result) = tokio::sync::oneshot::channel();
        if entry
            .request_sender
            .send(TransactionRequest::Commit { reply })
            .is_err()
        {
            let _ = self.event_sender.send(Action::ManualCommitFailed {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                message: "Commit acknowledgement was lost".to_owned(),
                unknown: true,
            });
            return;
        }
        let sender = self.event_sender.clone();
        let recorder = self.history_recorder.clone();
        let transaction_id = entry.transaction_id;
        self.background_tasks.push(tokio::spawn(async move {
            match result.await {
                Ok(Ok(())) => {
                    if let Some(recorder) = &recorder {
                        let _ = recorder
                            .resolve_transaction(
                                transaction_id,
                                crate::model::sql_history::HistoryTransactionOutcome::Committed,
                            )
                            .await;
                    }
                    let _ = sender.send(Action::ManualCommitted {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                    });
                }
                Ok(Err(error)) => {
                    let _ = sender.send(Action::ManualCommitFailed {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                        message: error.0,
                        unknown: false,
                    });
                }
                Err(_) => {
                    let _ = sender.send(Action::ManualCommitFailed {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                        message: "Commit acknowledgement was lost".to_owned(),
                        unknown: true,
                    });
                }
            }
        }));
    }

    fn manual_rollback(
        &mut self,
        connection: ConnectionIdentity,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
    ) {
        self.reap_finished_manual_worker(tab_id);
        let Some(entry) = self.manual_transactions.get(&tab_id) else {
            return;
        };
        let (reply, result) = tokio::sync::oneshot::channel();
        if entry
            .request_sender
            .send(TransactionRequest::Rollback { reply })
            .is_err()
        {
            let _ = self.event_sender.send(Action::ManualRollbackFailed {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                message: "Rollback acknowledgement was lost".to_owned(),
                unknown: true,
            });
            return;
        }
        let sender = self.event_sender.clone();
        let recorder = self.history_recorder.clone();
        let transaction_id = entry.transaction_id;
        self.background_tasks.push(tokio::spawn(async move {
            match result.await {
                Ok(Ok(())) => {
                    if let Some(recorder) = &recorder {
                        let _ = recorder
                            .resolve_transaction(
                                transaction_id,
                                crate::model::sql_history::HistoryTransactionOutcome::RolledBack,
                            )
                            .await;
                    }
                    let _ = sender.send(Action::ManualRolledBack {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                    });
                }
                Ok(Err(error)) => {
                    let _ = sender.send(Action::ManualRollbackFailed {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                        message: error.0,
                        unknown: false,
                    });
                }
                Err(_) => {
                    let _ = sender.send(Action::ManualRollbackFailed {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                        message: "Rollback acknowledgement was lost".to_owned(),
                        unknown: true,
                    });
                }
            }
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn ensure_manual_worker_page(
        &mut self,
        connection: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        _query_generation: u64,
        transaction_generation: u64,
        source_sql: String,
        dialect: crate::sql::SqlDialect,
        count_sql: String,
        page: crate::model::pagination::PageRequest,
        reply: tokio::sync::oneshot::Sender<
            Result<
                (
                    crate::db::query::QueryOutcome,
                    crate::model::pagination::ResultPagination,
                ),
                crate::db::transaction::TransactionError,
            >,
        >,
    ) {
        let Some(entry) = self.manual_transactions.get(&tab_id) else {
            let _ = reply.send(Err(crate::db::transaction::TransactionError(
                "Manual transaction worker is not active".into(),
            )));
            return;
        };
        if entry.connection != connection
            || entry.target != target
            || entry.transaction_generation != transaction_generation
        {
            let _ = reply.send(Err(crate::db::transaction::TransactionError(
                "Manual transaction does not match the execution target".into(),
            )));
            return;
        }
        let _ = entry.request_sender.send(TransactionRequest::Page {
            source_sql,
            dialect,
            count_sql,
            page,
            reply,
        });
    }

    fn ensure_manual_worker(
        &mut self,
        connection: ConnectionIdentity,
        target: ExecutionTarget,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
        request: Option<(
            String,
            tokio::sync::oneshot::Receiver<()>,
            tokio::sync::oneshot::Sender<
                Result<crate::db::query::QueryOutcome, crate::db::transaction::TransactionError>,
            >,
        )>,
    ) {
        if let Some(entry) = self.manual_transactions.get(&tab_id) {
            let matches = entry.connection == connection
                && entry.target == target
                && entry.transaction_generation == transaction_generation;
            match (matches, request) {
                (true, Some((sql, cancel, reply))) => {
                    let _ = entry.request_sender.send(TransactionRequest::Execute {
                        query_generation,
                        sql,
                        cancel,
                        reply,
                    });
                }
                (true, None) => {}
                (false, Some((_sql, _cancel, reply))) => {
                    let message = "Manual transaction does not match the execution target";
                    let _ = reply.send(Err(crate::db::transaction::TransactionError(
                        message.to_owned(),
                    )));
                }
                (false, None) => {
                    let _ = self.event_sender.send(Action::ManualStartFailed {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                        message: "Manual transaction does not match the execution target"
                            .to_owned(),
                    });
                }
            }
            return;
        }
        let (proxy, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let active_connection = Arc::clone(&self.connection);
        let sender = self.event_sender.clone();
        let worker_target = target.clone();
        let forced_close = ForcedCloseHandle::new();
        let transaction_id = Uuid::new_v4();
        let worker_forced_close = forced_close.clone();
        let worker_handle = tokio::spawn(async move {
            let Some(database) = active_database_for_target(
                Arc::clone(&active_connection),
                connection,
                &worker_target,
            )
            .await
            else {
                let _ = sender.send(Action::ManualStartFailed {
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    message: "No active database connection".to_owned(),
                });
                return crate::db::transaction::WorkerDisposition::Quarantine;
            };
            let quarantine_invalidates_connection = match &database {
                DatabaseConnection::Sqlite(_) => true,
                DatabaseConnection::Postgres(_)
                | DatabaseConnection::MySql(_)
                | DatabaseConnection::MariaDb(_)
                | DatabaseConnection::Oracle(_)
                | DatabaseConnection::SqlServer(_) => false,
            };
            let worker = match database
                .start_transaction_worker_with_forced_close(worker_forced_close)
                .await
            {
                Ok(worker) => worker,
                Err(error) => {
                    let _ = sender.send(Action::ManualStartFailed {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                        message: error.to_string(),
                    });
                    return crate::db::transaction::WorkerDisposition::Quarantine;
                }
            };
            let crate::runtime::transaction::TransactionWorkerHandle {
                requests, worker, ..
            } = worker;
            let _ = sender.send(Action::ManualStarted {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
            });
            tokio::pin!(worker);
            loop {
                tokio::select! {
                    request = receiver.recv() => {
                        let Some(request) = request else { break };
                        if requests.send(request).is_err() { break; }
                    }
                    disposition = &mut worker => {
                        let disposition = disposition.unwrap_or(crate::db::transaction::WorkerDisposition::Quarantine);
                        if disposition == crate::db::transaction::WorkerDisposition::ImplicitlyEnded {
                            let _ = sender.send(Action::ManualImplicitlyEnded { tab_id, query_generation, transaction_generation, connection });
                        }
                        if disposition == crate::db::transaction::WorkerDisposition::Quarantine
                            && quarantine_invalidates_connection
                        {
                            handle_quarantined_connection(
                                Arc::clone(&active_connection),
                                sender.clone(),
                                connection,
                            )
                            .await;
                        }
                        return disposition;
                    }
                }
            }
            let disposition = worker
                .await
                .unwrap_or(crate::db::transaction::WorkerDisposition::Quarantine);
            if disposition == crate::db::transaction::WorkerDisposition::Quarantine
                && quarantine_invalidates_connection
            {
                handle_quarantined_connection(Arc::clone(&active_connection), sender, connection)
                    .await;
            }
            disposition
        });
        self.manual_transactions.insert(
            tab_id,
            ManualTransactionEntry {
                connection,
                target,
                transaction_id,
                transaction_generation,
                request_sender: proxy,
                worker_handle,
                cancellation_sender: None,
                forced_close_handle: forced_close,
            },
        );
        if let Some((sql, cancel, reply)) = request
            && let Some(entry) = self.manual_transactions.get(&tab_id)
        {
            let _ = entry.request_sender.send(TransactionRequest::Execute {
                query_generation,
                sql,
                cancel,
                reply,
            });
        }
    }

    fn reap_finished_manual_worker(&mut self, tab_id: Uuid) {
        reap_finished_manual_worker(&mut self.manual_transactions, tab_id);
    }

    pub async fn shutdown(mut self) {
        if let Some(task) = self.update_startup_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.update_check_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.update_install_task.take() {
            task.abort();
            let _ = task.await;
        }
        for (_, task) in self.relation_tasks.drain() {
            task.abort();
            let _ = task.await;
        }
        for (_, task) in self.query_tasks.drain() {
            task.abort();
            let _ = task.await;
        }
        for (_, (_, _, task)) in self.catalog_search_tasks.drain() {
            task.abort();
            let _ = task.await;
        }
        for task in self.background_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        if let Some(queue) = self.workspace_save.as_mut()
            && let Some(task) = queue.active.take()
        {
            let _ = task.await;
        }
        for task in self.profile_tasks.drain(..) {
            let _ = task.await;
        }
        for (_, entry) in self.manual_transactions.drain() {
            let _ = entry
                .request_sender
                .send(crate::db::transaction::TransactionRequest::Shutdown);
            let forced_close = entry.forced_close_handle.clone();
            let mut worker = entry.worker_handle;
            if timeout(Duration::from_secs(2), &mut worker).await.is_err() {
                worker.abort();
                let _ = worker.await;
                let _ = timeout(Duration::from_secs(2), async {
                    while !forced_close.completed() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await;
            }
        }
        for (_, entry) in self.relation_transactions.drain() {
            let _ = entry
                .request_sender
                .send(crate::db::transaction::TransactionRequest::Shutdown);
            let _ = entry.worker_handle.await;
        }
        for connection in self
            .connection
            .lock()
            .await
            .drain()
            .map(|(_, connection)| connection)
        {
            connection.database.close().await;
        }
        if let Some(recorder) = self.history_recorder.take() {
            let _ = recorder.flush().await;
            let _ = recorder.shutdown().await;
        }
    }
}

async fn shutdown_transaction_entry(entry: ManualTransactionEntry) {
    let _ = entry
        .request_sender
        .send(crate::db::transaction::TransactionRequest::Shutdown);
    let forced_close = entry.forced_close_handle.clone();
    let mut worker = entry.worker_handle;
    if timeout(Duration::from_secs(2), &mut worker).await.is_err() {
        worker.abort();
        let _ = worker.await;
        let _ = timeout(Duration::from_secs(2), async {
            while !forced_close.completed() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
    }
}

fn reconcile_known_relations(
    known_relations: &StdMutex<HashSet<(ConnectionIdentity, crate::db::catalog::CatalogId)>>,
    known_relation_targets: &StdMutex<
        HashMap<
            (ConnectionIdentity, crate::db::catalog::CatalogId),
            crate::db::catalog::CatalogTarget,
        >,
    >,
    request: &crate::db::catalog::CatalogRequest,
    page: &crate::db::catalog::CatalogPage,
) {
    let Some(relation_target) = relation_target_for(&request.key.target) else {
        return;
    };
    let Some(mut known) = known_relations.lock().ok() else {
        return;
    };
    let Some(mut targets) = known_relation_targets.lock().ok() else {
        return;
    };
    // A relation-children response is only admissible while its owning relation
    // is still known for this connection. This prevents a delayed response for
    // an old OID from re-opening the old identity after a replacement page wins.
    if matches!(
        request.key.target,
        crate::db::catalog::CatalogTarget::RelationChildren { .. }
    ) && !known.contains(&(request.key.connection, relation_target.clone()))
    {
        return;
    }
    if request.key.cursor.is_none() {
        let stale = targets
            .iter()
            .filter(|(key, target)| {
                key.0 == request.key.connection && **target == request.key.target
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in stale {
            known.remove(&key);
            targets.remove(&key);
        }
    }
    for entry in page.entries.iter().filter(|entry| entry.kind.is_relation()) {
        let key = (request.key.connection, entry.id.clone());
        known.insert(key.clone());
        targets.insert(key, request.key.target.clone());
    }
}

fn reconcile_catalog_relation(
    known_relations: &StdMutex<HashSet<(ConnectionIdentity, crate::db::catalog::CatalogId)>>,
    known_relation_targets: &StdMutex<
        HashMap<
            (ConnectionIdentity, crate::db::catalog::CatalogId),
            crate::db::catalog::CatalogTarget,
        >,
    >,
    connection: ConnectionIdentity,
    old_relation: crate::db::catalog::CatalogId,
    new_relation: crate::db::catalog::CatalogId,
) {
    let Ok(mut known) = known_relations.lock() else {
        return;
    };
    let Ok(mut targets) = known_relation_targets.lock() else {
        return;
    };
    let old_key = (connection, old_relation);
    let target = targets.remove(&old_key);
    known.remove(&old_key);
    let new_key = (connection, new_relation);
    known.insert(new_key.clone());
    if let Some(target) = target {
        targets.insert(new_key, target);
    }
}

fn relation_target_for(
    target: &crate::db::catalog::CatalogTarget,
) -> Option<crate::db::catalog::CatalogId> {
    match target {
        crate::db::catalog::CatalogTarget::Objects { schema, group }
            if [
                crate::db::catalog::ObjectGroup::Tables,
                crate::db::catalog::ObjectGroup::Views,
                crate::db::catalog::ObjectGroup::MaterializedViews,
            ]
            .contains(group) =>
        {
            Some(schema.clone())
        }
        crate::db::catalog::CatalogTarget::RelationChildren { relation } => Some(relation.clone()),
        _ => None,
    }
}

async fn execute_catalog_drop_on_maintenance(
    plan: crate::db::catalog_drop::CatalogDropPlan,
    maintenance_database: String,
    connection: Arc<Mutex<HashMap<ConnectionKey, ActiveConnection>>>,
    registry: Arc<Mutex<ProfileRegistry>>,
    secret_store: Arc<dyn SecretStore>,
    local_credential_store: LocalCredentialStore,
    sender: mpsc::UnboundedSender<Action>,
) {
    if let Err(error) = plan.validate() {
        let _ = sender.send(Action::CatalogDropFailed {
            plan,
            message: error.to_string(),
        });
        return;
    }
    let profile = registry
        .lock()
        .await
        .profiles
        .get(&plan.request.connection.profile_id)
        .cloned();
    let Some(profile) = profile else {
        let _ = sender.send(Action::CatalogDropFailed {
            plan,
            message: "catalog drop profile is no longer active".to_owned(),
        });
        return;
    };
    if profile.read_only {
        let _ = sender.send(Action::CatalogDropFailed {
            plan,
            message: "catalog drop is unavailable on a read-only profile".to_owned(),
        });
        return;
    }
    let target = ExecutionTarget {
        profile_id: profile.id,
        database: maintenance_database,
        schema: None,
    };
    if !target.is_valid(&profile) {
        let _ = sender.send(Action::CatalogDropFailed {
            plan,
            message: "catalog drop maintenance database is invalid for this profile".to_owned(),
        });
        return;
    }
    let password =
        match resolve_profile_password(&registry, &secret_store, &local_credential_store, &profile)
            .await
        {
            Ok(password) => password,
            Err(error) => {
                let _ = sender.send(Action::CatalogDropFailed {
                    plan,
                    message: sanitize_terminal_text(&error.to_string()),
                });
                return;
            }
        };
    let maintenance =
        match DatabaseConnection::connect_target(&profile, password.as_ref(), &target).await {
            Ok(database) => database,
            Err(error) => {
                let _ = sender.send(Action::CatalogDropFailed {
                    plan,
                    message: format!(
                        "unable to connect to PostgreSQL maintenance database: {}",
                        sanitize_terminal_text(&error.to_string())
                    ),
                });
                return;
            }
        };
    let expected = plan.request.connection;
    let active = {
        let mut guard = connection.lock().await;
        guard
            .keys()
            .find(|key| key.identity == expected)
            .cloned()
            .and_then(|key| guard.remove(&key))
    };
    let Some(active) = active else {
        maintenance.close().await;
        let _ = sender.send(Action::CatalogDropFailed {
            plan,
            message: "catalog drop connection is no longer active".to_owned(),
        });
        return;
    };
    active.database.close().await;
    match maintenance.execute(plan.sql()).await {
        Ok(outcome) => {
            let _ = sender.send(Action::CatalogDropSucceeded {
                plan: plan.clone(),
                outcome,
            });
        }
        Err(error) => {
            let _ = sender.send(Action::CatalogDropFailed {
                plan: plan.clone(),
                message: sanitize_terminal_text(&error.to_string()),
            });
        }
    }
    let _ = sender.send(Action::DisconnectCompleted {
        connection: expected,
    });
    maintenance.close().await;
}

fn count_from_outcome(
    outcome: &crate::db::query::QueryOutcome,
) -> std::result::Result<u64, crate::db::transaction::TransactionError> {
    let value = outcome
        .result_sets
        .first()
        .and_then(|set| set.rows.first())
        .and_then(|row| row.first())
        .ok_or_else(|| {
            crate::db::transaction::TransactionError("count query returned no count value".into())
        })?;
    match value {
        crate::db::value::CellValue::Integer(value) => (*value).try_into().map_err(|_| {
            crate::db::transaction::TransactionError("count query returned a negative value".into())
        }),
        crate::db::value::CellValue::Unsigned(value) => Ok(*value),
        _ => Err(crate::db::transaction::TransactionError(
            "count query returned a non-integer value".into(),
        )),
    }
}

struct SavedProfile {
    profile: ConnectionProfile,
    warning: Option<String>,
    change: ProfileChange,
}

async fn save_profile_transaction(
    registry: Arc<Mutex<ProfileRegistry>>,
    profile_store: ProfileStore,
    secret_store: Arc<dyn SecretStore>,
    local_credential_store: LocalCredentialStore,
    submission: ProfileSubmission,
) -> Result<SavedProfile, String> {
    let snapshot = registry.lock().await.clone();
    let ProfileSubmission {
        mut profile,
        credential,
        ..
    } = submission;
    let profile_id = profile.id;
    let old_profile = snapshot.profiles.get(&profile_id).cloned();
    let mut next = snapshot;
    let mut previous_secret = None;
    let mut warning = None;
    let credentials_changed = match &credential {
        CredentialUpdate::Preserve => false,
        CredentialUpdate::Session(_)
        | CredentialUpdate::Remember(_)
        | CredentialUpdate::LocalEncrypted(_)
        | CredentialUpdate::System(_) => true,
        CredentialUpdate::Forget => old_profile
            .as_ref()
            .is_some_and(|old| !matches!(old.credential_policy, CredentialPolicy::None)),
    };

    match credential {
        CredentialUpdate::Preserve => {
            profile.credential_policy = if let Some(old_profile) = old_profile.as_ref() {
                validate_secret_reference(old_profile)?;
                old_profile.credential_policy.clone()
            } else {
                CredentialPolicy::None
            };
        }
        CredentialUpdate::Session(password) => {
            if let Some(old_profile) = old_profile.as_ref()
                && old_profile.credential_policy.keyring_reference().is_some()
            {
                validate_secret_reference(old_profile)?;
                let previous = read_secret(&secret_store, profile_id)
                    .await
                    .map_err(|error| secret_error("Unable to read the previous password", error))?;
                delete_secret(&secret_store, profile_id)
                    .await
                    .map_err(|error| {
                        secret_error("Unable to forget the previous password", error)
                    })?;
                previous_secret = Some(previous);
            }
            profile.credential_policy = CredentialPolicy::Prompt;
            next.session_secrets.insert(profile_id, password);
        }
        CredentialUpdate::LocalEncrypted(password) => {
            let encrypted = local_credential_store
                .encrypt(profile_id, &password)
                .map_err(|error| {
                    sanitize_terminal_text(&format!(
                        "Unable to encrypt the local password: {error}"
                    ))
                })?;
            if let Some(old_profile) = old_profile.as_ref()
                && old_profile.credential_policy.keyring_reference().is_some()
            {
                validate_secret_reference(old_profile)?;
                let previous = read_secret(&secret_store, profile_id)
                    .await
                    .map_err(|error| secret_error("Unable to read the previous password", error))?;
                delete_secret(&secret_store, profile_id)
                    .await
                    .map_err(|error| {
                        secret_error("Unable to forget the previous password", error)
                    })?;
                previous_secret = Some(previous);
            }
            profile.credential_policy = CredentialPolicy::LocalEncrypted(encrypted);
            next.session_secrets.insert(profile_id, password);
        }
        CredentialUpdate::Remember(password) => {
            if let Some(old_profile) = old_profile.as_ref()
                && old_profile.credential_policy.keyring_reference().is_some()
            {
                validate_secret_reference(old_profile)?;
            }
            match remember_secret(&secret_store, profile_id, &password).await? {
                RememberResult::Stored { previous } => {
                    previous_secret = Some(previous);
                    profile.credential_policy = CredentialPolicy::System(keyring_ref(profile_id));
                }
                RememberResult::SessionOnly => {
                    profile.credential_policy = CredentialPolicy::Prompt;
                    warning = Some(
                        if old_profile.as_ref().is_some_and(|profile| {
                            profile.credential_policy.keyring_reference().is_some()
                        }) {
                            "Native password store is unavailable; the password is session-only and the previous stored password could not be removed"
                            .to_owned()
                        } else {
                            "Native password store is unavailable; the password is available for this session only"
                            .to_owned()
                        },
                    );
                }
            }
            next.session_secrets.insert(profile_id, password);
        }
        CredentialUpdate::System(password) => {
            if let Some(old_profile) = old_profile.as_ref()
                && old_profile.credential_policy.keyring_reference().is_some()
            {
                validate_secret_reference(old_profile)?;
            }
            match remember_secret(&secret_store, profile_id, &password).await? {
                RememberResult::Stored { previous } => {
                    previous_secret = Some(previous);
                    profile.credential_policy = CredentialPolicy::System(keyring_ref(profile_id));
                }
                RememberResult::SessionOnly => {
                    profile.credential_policy = CredentialPolicy::LocalEncrypted(
                        local_credential_store
                            .encrypt(profile_id, &password)
                            .map_err(|error| {
                                sanitize_terminal_text(&format!(
                                    "Unable to encrypt the local password after native store failure: {error}"
                                ))
                            })?,
                    );
                    warning = Some(
                        "Native password store is unavailable; the password was saved using local encryption"
                            .to_owned(),
                    );
                }
            }
            next.session_secrets.insert(profile_id, password);
        }
        CredentialUpdate::Forget => {
            if let Some(old_profile) = old_profile.as_ref()
                && old_profile.credential_policy.keyring_reference().is_some()
            {
                validate_secret_reference(old_profile)?;
                let previous = read_secret(&secret_store, profile_id)
                    .await
                    .map_err(|error| secret_error("Unable to read the previous password", error))?;
                delete_secret(&secret_store, profile_id)
                    .await
                    .map_err(|error| secret_error("Unable to forget the password", error))?;
                previous_secret = Some(previous);
            }
            profile.credential_policy = CredentialPolicy::None;
            next.session_secrets.remove(&profile_id);
        }
    }

    let change = ProfileChange {
        connection_settings_changed: old_profile.as_ref().is_none_or(|old| {
            old.kind != profile.kind
                || old.host != profile.host
                || old.port != profile.port
                || old.user != profile.user
                || old.database != profile.database
                || old.default_schema != profile.default_schema
                || old.sqlite_path != profile.sqlite_path
                || old.ssl_mode != profile.ssl_mode
        }),
        catalog_scope_changed: old_profile
            .as_ref()
            .is_none_or(|old| old.catalog_scope != profile.catalog_scope),
        display_only_changed: old_profile.as_ref().is_none_or(|old| {
            old.name != profile.name
                || old.read_only != profile.read_only
                || old.environment != profile.environment
        }),
        credentials_changed,
    };

    if !next.order.contains(&profile_id) {
        next.order.push(profile_id);
    }
    next.profiles.insert(profile_id, profile.clone());
    *next.revisions.entry(profile_id).or_default() += 1;
    next.persisted.insert(profile_id);
    let collection = next.ordered_persisted_collection();
    if let Err(primary) = save_profiles(profile_store, collection).await {
        return Err(
            rollback_after_failure(&secret_store, profile_id, previous_secret, primary).await,
        );
    }

    *registry.lock().await = next;
    Ok(SavedProfile {
        profile,
        warning,
        change,
    })
}

async fn delete_profile_transaction(
    registry: Arc<Mutex<ProfileRegistry>>,
    profile_store: ProfileStore,
    secret_store: Arc<dyn SecretStore>,
    connection: Arc<Mutex<HashMap<ConnectionKey, ActiveConnection>>>,
    profile_id: Uuid,
) -> Result<Option<ConnectionIdentity>, String> {
    let snapshot = registry.lock().await.clone();
    let profile = snapshot
        .profiles
        .get(&profile_id)
        .cloned()
        .ok_or_else(|| "Connection profile no longer exists".to_owned())?;
    let mut previous_secret = None;
    if profile.credential_policy.keyring_reference().is_some() {
        validate_secret_reference(&profile)?;
        let previous = read_secret(&secret_store, profile_id)
            .await
            .map_err(|error| secret_error("Unable to read the stored password", error))?;
        delete_secret(&secret_store, profile_id)
            .await
            .map_err(|error| secret_error("Unable to delete the stored password", error))?;
        previous_secret = Some(previous);
    }

    let mut next = snapshot;
    next.order.retain(|id| *id != profile_id);
    next.profiles.remove(&profile_id);
    next.revisions.remove(&profile_id);
    let was_persisted = next.persisted.remove(&profile_id);
    next.session_secrets.remove(&profile_id);
    if next.startup_password_profile == Some(profile_id) {
        next.startup_password_profile = None;
        next.startup_password = None;
    }

    if was_persisted {
        let collection = next.ordered_persisted_collection();
        if let Err(primary) = save_profiles(profile_store, collection).await {
            return Err(rollback_after_failure(
                &secret_store,
                profile_id,
                previous_secret,
                primary,
            )
            .await);
        }
    }

    let active_connection = connection
        .lock()
        .await
        .iter()
        .find(|(key, _)| key.identity.profile_id == profile_id)
        .map(|(key, _)| key.identity);
    *registry.lock().await = next;
    Ok(active_connection)
}

impl ProfileRegistry {
    fn ordered_collection(&self) -> ProfileCollection {
        ProfileCollection {
            groups: self.groups.clone(),
            profiles: self
                .order
                .iter()
                .filter_map(|profile_id| self.profiles.get(profile_id).cloned())
                .collect(),
        }
    }

    fn ordered_persisted_profiles(&self) -> Vec<ConnectionProfile> {
        let mut seen = HashSet::new();
        self.order
            .iter()
            .filter(|profile_id| self.persisted.contains(profile_id))
            .filter(|profile_id| seen.insert(**profile_id))
            .filter_map(|profile_id| self.profiles.get(profile_id).cloned())
            .collect()
    }

    fn ordered_persisted_collection(&self) -> ProfileCollection {
        ProfileCollection {
            groups: self.groups.clone(),
            profiles: self.ordered_persisted_profiles(),
        }
    }
}

enum RememberResult {
    Stored { previous: Option<SecretString> },
    SessionOnly,
}

async fn remember_secret(
    secret_store: &Arc<dyn SecretStore>,
    profile_id: Uuid,
    password: &SecretString,
) -> Result<RememberResult, String> {
    match secret_store.available().await {
        Ok(()) => {}
        Err(error) if secret_store_unavailable(error) => return Ok(RememberResult::SessionOnly),
        Err(error) => {
            return Err(secret_error(
                "Unable to access the native password store",
                error,
            ));
        }
    }
    let previous = match read_secret(secret_store, profile_id).await {
        Ok(previous) => previous,
        Err(error) if secret_store_unavailable(error) => return Ok(RememberResult::SessionOnly),
        Err(error) => return Err(secret_error("Unable to read the previous password", error)),
    };
    match secret_store.set(profile_id, password).await {
        Ok(()) => Ok(RememberResult::Stored { previous }),
        Err(error) if secret_store_unavailable(error) => Ok(RememberResult::SessionOnly),
        Err(error) => Err(secret_error("Unable to remember the password", error)),
    }
}

async fn resolve_submission_password(
    registry: &Arc<Mutex<ProfileRegistry>>,
    secret_store: &Arc<dyn SecretStore>,
    local_credential_store: &LocalCredentialStore,
    profile: &ConnectionProfile,
    credential: CredentialUpdate,
) -> Result<Option<SecretString>, String> {
    match credential {
        CredentialUpdate::Session(password) | CredentialUpdate::Remember(password) => {
            Ok(Some(password))
        }
        CredentialUpdate::LocalEncrypted(password) | CredentialUpdate::System(password) => {
            Ok(Some(password))
        }
        CredentialUpdate::Forget => Ok(None),
        CredentialUpdate::Preserve => {
            resolve_profile_password(registry, secret_store, local_credential_store, profile).await
        }
    }
}

async fn discover_profile_catalog(
    profile: &ConnectionProfile,
    password: Option<&SecretString>,
) -> Result<
    (
        crate::db::ServerInfo,
        crate::db::catalog::CatalogCapabilities,
        CatalogDiscovery,
    ),
    String,
> {
    let connection = DatabaseConnection::connect(profile, password)
        .await
        .map_err(|error| sanitize_terminal_text(&error.to_string()))?;
    let server = match connection.probe().await {
        Ok(server) => server,
        Err(error) => {
            connection.close().await;
            return Err(sanitize_terminal_text(&error.to_string()));
        }
    };
    let capabilities = connection.catalog_capabilities();
    let postgres_databases = match connection.discoverable_postgres_databases().await {
        Ok(databases) => databases,
        Err(error) => {
            connection.close().await;
            return Err(sanitize_terminal_text(&error.to_string()));
        }
    };
    let discovery = if let Some(databases) = postgres_databases {
        connection.close().await;
        let profile = profile.clone();
        let password = password.cloned();
        let results = futures_util::stream::iter(databases.into_iter().map(|database| {
            let mut database_profile = profile.clone();
            let password = password.clone();
            async move {
                database_profile.database = Some(database.clone());
                let operation = async {
                    let connection =
                        DatabaseConnection::connect(&database_profile, password.as_ref()).await?;
                    let result = connection.discover_catalog_scope().await;
                    connection.close().await;
                    result
                };
                let result = timeout(Duration::from_secs(15), operation).await;
                (database, result)
            }
        }))
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
        let mut discovered = Vec::new();
        let mut warnings = Vec::new();
        for (database, result) in results {
            match result {
                Ok(Ok(scope)) => {
                    let schemas = scope
                        .databases
                        .into_iter()
                        .find(|item| item.name == database)
                        .map(|item| item.schemas)
                        .unwrap_or_default();
                    discovered.push(DiscoveredDatabase {
                        name: database,
                        schemas,
                    });
                    warnings.extend(scope.warnings);
                }
                Ok(Err(error)) => {
                    warnings.push(format!(
                        "{database}: {}",
                        sanitize_terminal_text(&error.to_string())
                    ));
                    discovered.push(DiscoveredDatabase {
                        name: database,
                        schemas: Vec::new(),
                    });
                }
                Err(_) => {
                    warnings.push(format!("{database}: discovery timed out after 15 seconds"));
                    discovered.push(DiscoveredDatabase {
                        name: database,
                        schemas: Vec::new(),
                    });
                }
            }
        }
        discovered.sort_by(|left, right| left.name.cmp(&right.name));
        CatalogDiscovery {
            databases: discovered,
            warnings,
        }
    } else {
        let result = connection
            .discover_catalog_scope()
            .await
            .map_err(|error| sanitize_terminal_text(&error.to_string()));
        connection.close().await;
        result?
    };
    Ok((server, capabilities, discovery))
}

async fn resolve_profile_password(
    registry: &Arc<Mutex<ProfileRegistry>>,
    secret_store: &Arc<dyn SecretStore>,
    local_credential_store: &LocalCredentialStore,
    profile: &ConnectionProfile,
) -> Result<Option<SecretString>, String> {
    let (session_password, startup_password) = {
        let registry = registry.lock().await;
        let session_password = registry.session_secrets.get(&profile.id).cloned();
        let startup_password = (registry.startup_password_profile == Some(profile.id))
            .then(|| registry.startup_password.clone())
            .flatten();
        (session_password, startup_password)
    };
    if session_password.is_some() {
        return Ok(session_password);
    }
    if startup_password.is_some() {
        return Ok(startup_password);
    }
    let resolver = CredentialResolver::new(secret_store.clone(), local_credential_store.clone());
    let password = resolver
        .resolve_headless(profile)
        .await
        .map_err(|error| match error {
            crate::persistence::credentials::CredentialResolutionError::InteractionRequired => {
                "Enter a password to continue".to_owned()
            }
            crate::persistence::credentials::CredentialResolutionError::Missing => {
                "Stored password is missing; enter a password to continue".to_owned()
            }
            crate::persistence::credentials::CredentialResolutionError::Unavailable => {
                "Stored password is unavailable; enter a password to continue".to_owned()
            }
            other => sanitize_terminal_text(&other.to_string()),
        })?;
    if let Some(password) = &password {
        registry
            .lock()
            .await
            .session_secrets
            .insert(profile.id, password.clone());
    }
    Ok(password)
}

fn validate_secret_reference(profile: &ConnectionProfile) -> Result<(), String> {
    let Some(reference) = profile.credential_policy.keyring_reference() else {
        return Ok(());
    };
    let referenced_profile = profile_id_from_ref(reference)
        .map_err(|error| secret_error("Invalid stored password reference", error))?;
    if referenced_profile != profile.id {
        return Err("Invalid stored password reference".to_owned());
    }
    Ok(())
}

async fn read_secret(
    secret_store: &Arc<dyn SecretStore>,
    profile_id: Uuid,
) -> Result<Option<SecretString>, SecretStoreError> {
    match secret_store.get(profile_id).await {
        Err(SecretStoreError::Missing) => Ok(None),
        result => result,
    }
}

async fn delete_secret(
    secret_store: &Arc<dyn SecretStore>,
    profile_id: Uuid,
) -> Result<(), SecretStoreError> {
    match secret_store.delete(profile_id).await {
        Err(SecretStoreError::Missing) => Ok(()),
        result => result,
    }
}

async fn rollback_after_failure(
    secret_store: &Arc<dyn SecretStore>,
    profile_id: Uuid,
    previous_secret: Option<Option<SecretString>>,
    primary: String,
) -> String {
    let Some(previous_secret) = previous_secret else {
        return primary;
    };
    let rollback = match previous_secret {
        Some(password) => secret_store.set(profile_id, &password).await,
        None => delete_secret(secret_store, profile_id).await,
    };
    match rollback {
        Ok(()) => primary,
        Err(error) => format!(
            "{primary}; restoring the previous password also failed: {}",
            sanitize_terminal_text(&error.to_string())
        ),
    }
}

async fn save_profiles(
    profile_store: ProfileStore,
    collection: ProfileCollection,
) -> Result<(), String> {
    task::spawn_blocking(move || profile_store.save(&collection))
        .await
        .map_err(|_| "Profile persistence task failed".to_owned())?
        .map_err(|error| {
            sanitize_terminal_text(&format!("Unable to save connection profiles: {error}"))
        })
}

fn secret_store_unavailable(error: SecretStoreError) -> bool {
    matches!(
        error,
        SecretStoreError::Locked | SecretStoreError::Unavailable
    )
}

fn secret_error(context: &str, error: SecretStoreError) -> String {
    sanitize_terminal_text(&format!("{context}: {error}"))
}

fn local_credential_store_for(profile_store: &ProfileStore) -> LocalCredentialStore {
    let key_path = profile_store
        .path()
        .parent()
        .map(|parent| parent.join("credential.key"))
        .unwrap_or_else(|| std::path::PathBuf::from("credential.key"));
    LocalCredentialStore::new(key_path, "lazydb")
}

async fn profile_revision_is_current(
    registry: &Arc<Mutex<ProfileRegistry>>,
    profile: &ConnectionProfile,
    profile_revision: u64,
) -> bool {
    let registry = registry.lock().await;
    registry.profiles.get(&profile.id) == Some(profile)
        && registry.revisions.get(&profile.id) == Some(&profile_revision)
}

fn connection_attempt_is_current(
    attempts: &StdMutex<ConnectionAttempts>,
    expected: &ConnectionKey,
) -> bool {
    let attempts = attempts.lock().expect("connection attempt mutex poisoned");
    attempts.is_current(expected)
}

async fn active_database_for_target(
    connection: Arc<Mutex<HashMap<ConnectionKey, ActiveConnection>>>,
    expected: ConnectionIdentity,
    target: &ExecutionTarget,
) -> Option<DatabaseConnection> {
    connection
        .lock()
        .await
        .get(&ConnectionKey::new(expected, target.clone()))
        .map(|active| active.database.clone())
}

async fn active_database(
    connection: Arc<Mutex<HashMap<ConnectionKey, ActiveConnection>>>,
    expected: ConnectionIdentity,
) -> Option<DatabaseConnection> {
    connection
        .lock()
        .await
        .iter()
        .find(|(key, _)| key.identity == expected)
        .map(|(_, active)| active.database.clone())
}

fn definition_baseline_fingerprint(
    definition: &crate::db::catalog_mutation::CatalogObjectDefinition,
) -> Option<String> {
    use crate::db::catalog_mutation::CatalogObjectDefinition;
    Some(match definition {
        CatalogObjectDefinition::Database(value) => value.baseline_fingerprint.clone(),
        CatalogObjectDefinition::Schema(value) => value.baseline_fingerprint.clone(),
        CatalogObjectDefinition::Table(value) => value.baseline_fingerprint.clone(),
        CatalogObjectDefinition::Index(value) => value.baseline_fingerprint.clone(),
        CatalogObjectDefinition::Constraint(value) => value.baseline_fingerprint.clone(),
        CatalogObjectDefinition::View(value) => value.baseline_fingerprint.clone(),
        CatalogObjectDefinition::MaterializedView(value) => value.baseline_fingerprint.clone(),
        CatalogObjectDefinition::Sequence(value) => value.baseline_fingerprint.clone(),
        CatalogObjectDefinition::Role(value) => value.baseline_fingerprint.clone(),
    })
}

async fn resolve_catalog_mutation_connection(
    connection: Arc<Mutex<HashMap<ConnectionKey, ActiveConnection>>>,
    expected: ConnectionIdentity,
    target: crate::db::catalog_mutation::CatalogMutationTarget,
    registry: &Arc<Mutex<ProfileRegistry>>,
    secret_store: &Arc<dyn SecretStore>,
    local_credential_store: &LocalCredentialStore,
) -> Result<CatalogMutationConnection, String> {
    let profile = registry
        .lock()
        .await
        .profiles
        .get(&expected.profile_id)
        .cloned()
        .ok_or_else(|| "Mutation profile no longer exists".to_owned())?;
    let target = target.execution_target(profile.id);
    if !target.is_valid(&profile) {
        return Err("Catalog mutation target is invalid for this profile".to_owned());
    }
    let active_guard = connection.lock().await;
    if let Some(active) = active_guard.get(&ConnectionKey::new(expected, target.clone())) {
        return Ok(CatalogMutationConnection {
            database: active.database.clone(),
            owned: false,
        });
    }
    drop(active_guard);
    let password =
        resolve_profile_password(registry, secret_store, local_credential_store, &profile).await?;
    DatabaseConnection::connect_target(&profile, password.as_ref(), &target)
        .await
        .map(|database| CatalogMutationConnection {
            database,
            owned: true,
        })
        .map_err(|error| sanitize_terminal_text(&error.to_string()))
}

fn take_active_connection(
    active: &mut HashMap<ConnectionKey, ActiveConnection>,
    expected: ConnectionIdentity,
) -> Option<ActiveConnection> {
    active
        .keys()
        .find(|key| key.identity == expected)
        .cloned()
        .and_then(|key| active.remove(&key))
}

fn reap_finished_manual_worker(
    manual_transactions: &mut HashMap<Uuid, ManualTransactionEntry>,
    tab_id: Uuid,
) {
    let finished = manual_transactions
        .get(&tab_id)
        .is_some_and(|entry| entry.worker_handle.is_finished());
    if finished {
        manual_transactions.remove(&tab_id);
    }
}

async fn handle_quarantined_connection(
    connection: Arc<Mutex<HashMap<ConnectionKey, ActiveConnection>>>,
    sender: mpsc::UnboundedSender<Action>,
    expected: ConnectionIdentity,
) {
    let active = {
        let mut active = connection.lock().await;
        take_active_connection(&mut active, expected)
    };
    let Some(active) = active else {
        return;
    };
    active.database.close().await;
    let _ = sender.send(Action::ConnectionInvalidated {
        connection: expected,
        message: "The SQLite transaction worker was quarantined and closed this connection. Reconnect before running more queries. The transaction outcome is unknown."
            .to_owned(),
    });
}

pub async fn run_tui(cli: Cli) -> Result<RunOutcome> {
    let project = crate::project::ProjectContext::resolve_current()
        .context("failed to resolve current project")?;
    let startup = load_startup_profiles(&cli)?;
    let paths = AppPaths::discover()?;
    let mut settings = crate::persistence::settings::AppSettings::load(paths.settings_file())
        .context("failed to load application settings")?;
    settings.apply_cli_overrides(
        cli.mouse,
        cli.color,
        cli.icons,
        cli.motion,
        cli.confirm_execution,
    );
    let workspace_store = WorkspaceStore::new(paths.workspace_file(), paths.workspace_sql_dir());
    let workspace = workspace_store.load().context("failed to load workspace")?;
    let mut app = App::with_startup_project(
        startup.profiles.clone(),
        startup.persisted.clone(),
        settings.execution.confirmation,
        project,
    );
    app.set_default_connection_access(settings.connections.default_access);
    app.connection_groups = startup.collection.groups.clone();
    app.explorer.normalized.sync_organization(
        startup.collection.groups.clone(),
        app.profiles.iter().map(|profile| profile.id).collect(),
        &app.profiles
            .iter()
            .map(|profile| (profile.id, profile.group_id))
            .collect(),
    );
    app.add_unavailable_profiles(&startup.unavailable);
    if !startup.unavailable.is_empty() {
        app.notify_warning(
            "Profile",
            format!(
                "{} connection profile(s) are not supported by this version",
                startup.unavailable.len()
            ),
        );
    }
    if let Some(workspace) = workspace {
        app.restore_workspace(workspace, startup.selected);
    }
    app.set_dashboard_refresh_interval_millis(settings.dashboard_refresh_interval_millis());
    app.set_key_bindings(settings.keybindings.key_bindings()?);
    app.reveal_startup_profile(startup.selected);
    app.focus = crate::model::workspace::Focus::Explorer;
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new_with_collection(
        startup.collection,
        startup.persisted,
        startup.session_secrets,
        startup.startup_password,
        startup.profile_store,
        Arc::new(NativeSecretStore),
        event_sender,
    );
    let history_store = crate::persistence::sql_history::HistoryStore::open(paths.history_file())
        .await
        .context("failed to open SQL history database")?;
    runtime.set_history_recorder(crate::history::HistoryRecorder::default_capacity(
        history_store.clone(),
    ));
    runtime.set_history_store(history_store);
    runtime.set_workspace_store(workspace_store);
    runtime.set_clipboard_config(settings.terminal.clipboard);
    let mut terminal = TerminalSession::enter(settings.terminal.mouse != MouseMode::Off)
        .context("failed to initialize terminal")?;
    let mut terminal_selection_mode = false;
    let icons = crate::ui::icons::IconSet::new(settings.ui.icons);
    let mut theme = crate::ui::theme::Theme::for_color_mode(settings.terminal.color);
    let mut external_theme_source = cli
        .theme_file
        .clone()
        .filter(|_| settings.terminal.color != ColorMode::Never)
        .map(crate::ui::theme::external::ExternalThemeSource::new);
    if let Some(source) = external_theme_source.as_mut() {
        match source.check().await {
            crate::ui::theme::external::SourceOutcome::ThemeChanged(next) => theme = next,
            crate::ui::theme::external::SourceOutcome::Error(error) => {
                app.notify_warning("Theme", error.to_string());
            }
            crate::ui::theme::external::SourceOutcome::Unchanged => {}
        }
    }
    let mut terminal_events = EventStream::new();
    let mut keymap = Keymap::with_sequence_timeout_and_bindings(
        Duration::from_millis(settings.keybindings.sequence_timeout_ms),
        settings
            .keybindings
            .key_bindings()
            .context("failed to parse application keybindings")?,
    );
    let mut ui_state = UiState::with_motion(settings.ui.motion);
    let mut rendered_sequence: Option<crate::input::keymap::KeySequenceState> = None;
    let mut ticker = interval(Duration::from_millis(33));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let (theme_sender, mut theme_receiver) = tokio::sync::watch::channel(ThemeUpdate::Theme(theme));
    let mut theme_task = cli
        .theme_file
        .clone()
        .filter(|_| settings.terminal.color != ColorMode::Never)
        .map(|path| tokio::spawn(poll_external_theme(path, theme_sender)));

    let result: Result<Option<std::path::PathBuf>> = async {
        apply_startup_action_with_runtime(&mut app, &mut runtime, startup.selected);
        runtime.dispatch(Command::CheckSecretStoreAvailability);
        let initial_sequence = keymap.sequence_state(&app, std::time::Instant::now());
        rendered_sequence = initial_sequence.clone();
        terminal.draw(|frame| {
            ui::render_with_state_using_icons_sequence_and_theme(
                frame,
                &app,
                &mut ui_state,
                icons,
                initial_sequence.as_ref(),
                theme,
            )
        })?;
        let startup_check_due =
            crate::persistence::update_check::UpdateCheckCache::read(&paths.update_check_file())
                .is_none_or(|cache| {
                    !cache.is_fresh(
                        std::time::SystemTime::now(),
                        Duration::from_secs(settings.update_check_interval_hours() * 60 * 60),
                    )
                });
        if settings.updates.check_on_startup && startup_check_due {
            runtime.dispatch(Command::ScheduleUpdateCheck { delay_ms: 1_500 });
        }
        if let Some(cursor) = ui_state.cursor {
            terminal.set_cursor_style(cursor.style)?;
        }
        sync_ddl_editor_viewport(&mut app, &mut runtime, terminal.size()?);
        sync_editor_viewport(&mut app, &mut runtime, &ui_state);
        sync_output_viewport(&mut app, &mut runtime, &ui_state);
        sync_pane_layout(&mut app, &mut runtime, &ui_state);
        sync_grid_viewport(&mut app, &mut runtime, &ui_state);
        sync_record_view_fields(&mut app, &mut runtime, &ui_state);
        sync_explorer_viewport(&mut app, &mut runtime, &ui_state);
        sync_ddl_viewport(&mut app, &mut runtime, &ui_state);

        while !app.should_quit {
            let mut redraw = false;
            tokio::select! {
                terminal_event = terminal_events.next() => {
                        let Some(terminal_event) = terminal_event else { break; };
                        match terminal_event.context("terminal input failed")? {
                        Event::Key(key) => {
                            let had_text_gesture = ui_state.text_gesture.borrow().is_some();
                            ui_state.cancel_mouse_gesture();
                            if terminal_selection_mode && key.code == crossterm::event::KeyCode::Esc {
                                match terminal.set_mouse_capture(true) {
                                            Ok(()) => {
                                                terminal_selection_mode = false;
                                                ui_state.terminal_selection_mode = false;
                                                redraw = true;
                                    }
                                    Err(error) => app.notify_warning(
                                        "Terminal",
                                        format!("Could not restore mouse capture: {error}"),
                                    ),
                                }
                            } else {
                            let now = std::time::Instant::now();
                            let before = keymap.sequence_state(&app, now);
                            let cancelled_pane_drag = ui_state.pane_resize_drag.borrow_mut().take().is_some();
                            if let Some(action) = keymap.map(key, &app) {
                                if action == Action::ToggleTerminalSelection {
                                    if !terminal.mouse_captured() {
                                        app.notify_warning(
                                            "Terminal",
                                            "Mouse capture is disabled (--mouse off)",
                                        );
                                    } else {
                                        match terminal.set_mouse_capture(false) {
                                            Ok(()) => {
                                                terminal_selection_mode = true;
                                                ui_state.terminal_selection_mode = true;
                                                app.notify_info(
                                                    "Terminal selection",
                                                    "Mouse released for terminal selection. Press Esc to return.",
                                                );
                                            }
                                            Err(error) => app.notify_warning(
                                                "Terminal",
                                                format!("Could not release mouse capture: {error}"),
                                            ),
                                        }
                                    }
                                } else {
                                    apply_action(&mut app, &mut runtime, action);
                                }
                                redraw = true;
                            }
                            redraw |= cancelled_pane_drag;
                            redraw |= had_text_gesture;
                            let after = keymap.sequence_state(&app, now);
                            redraw |= sequence_redraw_needed(&before, &after);
                            }
                        }
                        Event::Mouse(mouse) => {
                            let now = std::time::Instant::now();
                            let before = keymap.sequence_state(&app, now);
                            let is_move = matches!(mouse.kind, MouseEventKind::Moved);
                            if !is_move {
                                keymap.clear_pending();
                            }
                            let was_pane_drag = ui_state.pane_resize_drag.borrow().is_some();
                            let before_text_gesture = *ui_state.text_gesture.borrow();
                            if let Some(action) = map_mouse(mouse, &ui_state, &app) {
                                apply_action(&mut app, &mut runtime, action);
                                redraw = true;
                            }
                            redraw |= was_pane_drag
                                != ui_state.pane_resize_drag.borrow().is_some();
                            redraw |= before_text_gesture != *ui_state.text_gesture.borrow();
                            let after = keymap.sequence_state(&app, now);
                            redraw |= sequence_redraw_needed(&before, &after);
                        }
                        Event::Paste(value) => {
                            let now = std::time::Instant::now();
                            let before = keymap.sequence_state(&app, now);
                            keymap.clear_pending();
                            let had_text_gesture = ui_state.text_gesture.borrow().is_some();
                            ui_state.cancel_mouse_gesture();
                            let cancelled_pane_drag = ui_state.pane_resize_drag.borrow_mut().take().is_some();
                            let actions = map_paste(value, &app);
                            if !actions.is_empty() {
                                for action in actions {
                                    apply_action(&mut app, &mut runtime, action);
                                }
                                redraw = true;
                            }
                            redraw |= cancelled_pane_drag;
                            redraw |= had_text_gesture;
                            let after = keymap.sequence_state(&app, now);
                            redraw |= sequence_redraw_needed(&before, &after);
                        }
                        Event::Resize(_, _) | Event::FocusGained | Event::FocusLost => {
                            let now = std::time::Instant::now();
                            let before = keymap.sequence_state(&app, now);
                            keymap.clear_pending();
                            ui_state.cancel_mouse_gesture();
                            ui_state.pane_resize_drag.borrow_mut().take();
                            ui_state.grid_scrollbar_drag.borrow_mut().take();
                            ui_state.relation_resize.borrow_mut().take();
                            redraw = true;
                            let after = keymap.sequence_state(&app, now);
                            redraw |= sequence_redraw_needed(&before, &after);
                        }
                    }
                }
                Some(action) = event_receiver.recv() => {
                    let now = std::time::Instant::now();
                    let before = keymap.sequence_state(&app, now);
                    if let Action::Osc52Clipboard { payload } = action {
                        match terminal.write_osc52(&payload.text, settings.terminal.clipboard.max_bytes) {
                            Ok(()) => app.notify_info(
                                "Clipboard",
                                format!("Sent {} to terminal clipboard", payload.description),
                            ),
                            Err(error) => app.notify_error("Clipboard", error.to_string()),
                        }
                    } else {
                        apply_action(&mut app, &mut runtime, action);
                    }
                    redraw = true;
                    let after = keymap.sequence_state(&app, now);
                    redraw |= sequence_redraw_needed(&before, &after);
                }
                changed = theme_receiver.changed(), if theme_task.is_some() => {
                    if changed.is_ok() {
                        let (theme_changed, error) =
                            apply_theme_update(&mut theme, theme_receiver.borrow_and_update().clone());
                        redraw |= theme_changed;
                        if let Some(error) = error {
                            app.notify_warning("Theme", error);
                        }
                    }
                }
                _ = ticker.tick() => {
                    let now = std::time::Instant::now();
                    let expired = keymap.expire_pending(&app, now);
                    let after = keymap.sequence_state(&app, now);
                    let now_millis = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let refresh_commands = app.dashboard_refresh_commands(now_millis);
                    for command in refresh_commands {
                        runtime.dispatch(command);
                    }
                    redraw = app.notifications.expire(now)
                        || ui_state.advance_animations(now)
                        || expired
                        || sequence_redraw_needed(&rendered_sequence, &after);
                }
            }

            if redraw && !app.should_quit {
                let sequence = keymap.sequence_state(&app, std::time::Instant::now());
                rendered_sequence = sequence.clone();
                sync_ddl_editor_viewport(&mut app, &mut runtime, terminal.size()?);
                terminal.draw(|frame| {
                    ui::render_with_state_using_icons_sequence_and_theme(
                        frame,
                        &app,
                        &mut ui_state,
                        icons,
                        sequence.as_ref(),
                        theme,
                    )
                })?;
                if let Some(cursor) = ui_state.cursor {
                    terminal.set_cursor_style(cursor.style)?;
                }
                sync_editor_viewport(&mut app, &mut runtime, &ui_state);
                sync_output_viewport(&mut app, &mut runtime, &ui_state);
                sync_pane_layout(&mut app, &mut runtime, &ui_state);
                sync_grid_viewport(&mut app, &mut runtime, &ui_state);
                sync_record_view_fields(&mut app, &mut runtime, &ui_state);
                sync_explorer_viewport(&mut app, &mut runtime, &ui_state);
                sync_ddl_viewport(&mut app, &mut runtime, &ui_state);
                }
        }

        Ok(app.restart_path.take())
    }
    .await;

    runtime.shutdown().await;
    if let Some(theme_task) = theme_task.take() {
        theme_task.abort();
        let _ = theme_task.await;
    }
    drop(terminal);
    result.map(|restart_path| match restart_path {
        Some(executable) => RunOutcome::Restart { executable },
        None => RunOutcome::Exit,
    })
}

pub fn apply_startup_action(app: &mut App, selected: Option<Uuid>) {
    if let Some(profile_id) = selected {
        app.update(Action::RequestProfileConnect { profile_id });
    }
}

fn apply_startup_action_with_runtime(app: &mut App, runtime: &mut Runtime, selected: Option<Uuid>) {
    if let Some(profile_id) = selected {
        apply_action(app, runtime, Action::RequestProfileConnect { profile_id });
    }
}

fn apply_action(app: &mut App, runtime: &mut Runtime, action: Action) {
    let finished_search = match &action {
        Action::CatalogSearchSucceeded { owner, page } => {
            Some((*owner, page.session_id, page.generation))
        }
        Action::CatalogSearchFailed {
            owner,
            session_id,
            generation,
            ..
        } => Some((*owner, *session_id, *generation)),
        _ => None,
    };
    for command in app.update(action) {
        runtime.dispatch(command);
    }
    if let Some((owner, session_id, generation)) = finished_search {
        let key = (owner, session_id);
        if runtime
            .catalog_search_tasks
            .get(&key)
            .is_some_and(|(_, current_generation, _)| *current_generation == generation)
        {
            runtime.catalog_search_tasks.remove(&key);
        }
    }
}

#[cfg(test)]
mod key_sequence_redraw_tests {
    use super::*;

    fn state(display: &str) -> crate::input::keymap::KeySequenceState {
        crate::input::keymap::KeySequenceState {
            prefix: crate::help::ShortcutPrefix::Leader,
            display: display.to_owned(),
            selected: 0,
        }
    }

    #[test]
    fn sequence_redraw_needed_for_visible_state_transitions_only() {
        assert!(sequence_redraw_needed(&None, &Some(state("Space"))));
        assert!(sequence_redraw_needed(&Some(state("Space")), &None));
        assert!(!sequence_redraw_needed(&None, &None));
        assert!(!sequence_redraw_needed(
            &Some(state("Space")),
            &Some(state("Space"))
        ));
    }

    #[test]
    fn sequence_redraw_needed_is_used_for_completion_and_timeout_clear() {
        let prefix = Some(state("Space"));
        assert!(sequence_redraw_needed(&prefix, &None));
        assert!(!sequence_redraw_needed(&None, &None));
    }
}

#[cfg(test)]
mod theme_update_tests {
    use ratatui::style::Color;

    use super::{ThemeUpdate, apply_theme_update};
    use crate::ui::theme::Theme;

    #[test]
    fn applies_latest_theme_without_resetting_runtime_state() {
        let mut current = Theme::deep_space();
        let mut next = current;
        next.background = Color::Rgb(1, 2, 3);

        assert_eq!(
            apply_theme_update(&mut current, ThemeUpdate::Theme(next)),
            (true, None)
        );
        assert_eq!(current.background, Color::Rgb(1, 2, 3));
        assert_eq!(
            apply_theme_update(&mut current, ThemeUpdate::Theme(next)),
            (false, None)
        );
    }

    #[test]
    fn theme_errors_keep_the_current_theme() {
        let mut current = Theme::deep_space();
        let original = current;

        assert_eq!(
            apply_theme_update(&mut current, ThemeUpdate::Error("invalid".to_owned())),
            (false, Some("invalid".to_owned()))
        );
        assert_eq!(current, original);
    }
}

fn sequence_redraw_needed(
    previous: &Option<crate::input::keymap::KeySequenceState>,
    current: &Option<crate::input::keymap::KeySequenceState>,
) -> bool {
    previous != current
}

#[cfg(test)]
mod workspace_save_tests {
    use super::WorkspaceSaveQueue;
    use crate::action::Action;
    use crate::persistence::workspace::WorkspaceSnapshot;

    fn snapshot() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            active_profile: None,
            profiles: Vec::new(),
            sql: Vec::new(),
            active_console: uuid::Uuid::nil(),
            consoles: Vec::new(),
            tabs: Vec::new(),
            recent_targets: Vec::new(),
        }
    }

    #[test]
    fn offering_new_snapshots_keeps_only_the_latest_pending_revision() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::persistence::workspace::WorkspaceStore::new(
            temp.path().join("workspace.toml"),
            temp.path().join("sql"),
        );
        let mut queue = WorkspaceSaveQueue::new(store);
        queue.offer(1, snapshot());
        queue.offer(2, snapshot());

        assert_eq!(
            queue.pending.as_ref().map(|(revision, _)| *revision),
            Some(2)
        );
        assert_eq!(queue.next_revision, 2);
    }

    #[tokio::test]
    async fn failed_flush_does_not_emit_flushed_until_retry_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::persistence::workspace::WorkspaceStore::new(
            temp.path().join("workspace.toml"),
            temp.path().join("sql"),
        );
        let mut queue = WorkspaceSaveQueue::new(store);
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        queue.next_revision = 1;
        queue.failed = Some((1, snapshot()));
        queue.flushing = Some(1);

        assert!(receiver.try_recv().is_err());

        queue.retry(sender.clone());
        assert!(matches!(
            receiver.recv().await,
            Some(Action::WorkspaceSaveSucceeded { revision: 1 })
        ));
        queue.complete(1, true, sender);
        assert!(matches!(
            receiver.recv().await,
            Some(Action::WorkspaceSaveFlushed { revision: 1 })
        ));
    }
}

fn sync_ddl_editor_viewport(app: &mut App, runtime: &mut Runtime, area: ratatui::layout::Rect) {
    if let Some((session_id, viewport)) = ui::relation::ddl_editor_viewport(area, app)
        && app
            .active_ddl_editor_viewport()
            .ok()
            .is_none_or(|current| current != viewport)
    {
        apply_action(
            app,
            runtime,
            Action::DdlEditorViewportChanged {
                session_id,
                viewport,
            },
        );
    }
}

fn sync_editor_viewport(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    let Some(viewport) = state.editor_viewport else {
        return;
    };
    if app.focus == crate::model::workspace::Focus::Results
        && app.active_console_opt().is_some_and(|tab| {
            matches!(
                tab.result_view,
                crate::model::tab::ResultView::Output | crate::model::tab::ResultView::Plan
            )
        })
    {
        return;
    }
    let current = if app.focus == crate::model::workspace::Focus::Results
        && app.is_active_relation_tab()
        && app.tabs.get(app.active_tab).is_some_and(|tab| {
            matches!(
                tab,
                crate::model::tab::WorkspaceTab::Relation(relation)
                    if relation.view == crate::model::relation::RelationView::Ddl
            )
        }) {
        app.active_ddl_editor_viewport().ok()
    } else {
        app.active_editor_viewport().ok()
    };
    if current != Some(viewport) {
        apply_action(app, runtime, Action::EditorViewportChanged(viewport));
    }
}

fn sync_output_viewport(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    let Some((session_id, viewport)) = state.output_viewport else {
        return;
    };
    apply_action(
        app,
        runtime,
        Action::OutputViewportChanged {
            session_id,
            viewport,
        },
    );
}

fn sync_pane_layout(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    if app.pane_layout_metrics() != state.pane_layout {
        apply_action(app, runtime, Action::PaneLayoutChanged(state.pane_layout));
    }
}

fn sync_grid_viewport(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    let Some(viewport) = state.grid_viewport else {
        return;
    };
    apply_action(app, runtime, Action::GridViewportChanged(viewport));
}

fn sync_record_view_fields(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    let Some((tab_id, visible_fields)) = state.record_view_fields else {
        return;
    };
    apply_action(
        app,
        runtime,
        Action::RecordViewViewportChanged {
            tab_id,
            visible_fields,
        },
    );
}

fn sync_explorer_viewport(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    let Some(rows) = state.explorer_viewport_rows else {
        return;
    };
    if app.explorer.normalized.viewport_height != rows {
        apply_action(app, runtime, Action::ExplorerViewportChanged(rows));
    }
}

fn sync_ddl_viewport(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    let Some(metrics) = state.ddl_viewport else {
        return;
    };
    apply_action(
        app,
        runtime,
        Action::SetDdlViewportMetrics {
            visible_rows: metrics.visible_rows,
            visible_columns: metrics.visible_columns,
            total_rows: metrics.total_rows,
            max_line_width: metrics.max_line_width,
        },
    );
}

pub struct StartupProfiles {
    pub collection: ProfileCollection,
    pub unavailable: Vec<crate::profile_compatibility::UnavailableProfile>,
    pub profiles: Vec<ConnectionProfile>,
    pub persisted: HashSet<Uuid>,
    pub session_secrets: HashMap<Uuid, SecretString>,
    pub startup_password: Option<(Uuid, SecretString)>,
    pub selected: Option<Uuid>,
    pub profile_store: ProfileStore,
}

pub fn load_startup_profiles(cli: &Cli) -> Result<StartupProfiles> {
    let profile_path = if let Some(path) = &cli.config {
        path.clone()
    } else {
        AppPaths::discover()?.profiles_file()
    };
    let store = ProfileStore::new(profile_path);
    let report = store
        .load_report()
        .context("failed to load connection profiles")?;
    let mut collection = report.collection;
    let mut profiles = collection.profiles.clone();
    let persisted = profiles.iter().map(|profile| profile.id).collect();
    let mut session_secrets = HashMap::new();

    let direct_profile = if let Some(url) = &cli.url {
        let mut imported = import_connection_url(url, cli.profile.as_deref())?;
        if cli.read_only {
            imported.profile.read_only = true;
        }
        if let Some(password) = imported.transient_password {
            session_secrets.insert(imported.profile.id, password);
        }
        let profile_id = imported.profile.id;
        profiles.push(imported.profile);
        Some(profile_id)
    } else {
        None
    };
    collection.profiles = profiles.clone();

    let has_direct_profile = direct_profile.is_some();
    let selected = direct_profile.or_else(|| {
        cli.profile.as_deref().and_then(|name| {
            profiles
                .iter()
                .find(|profile| profile.name == name)
                .map(|profile| profile.id)
        })
    });
    if cli.profile.is_some() && !has_direct_profile && selected.is_none() {
        anyhow::bail!(
            "connection profile not found: {}",
            cli.profile.as_deref().unwrap_or_default()
        );
    }
    let startup_password = if session_secrets
        .keys()
        .copied()
        .any(|profile_id| Some(profile_id) == selected)
    {
        None
    } else {
        std::env::var("LAZYDB_PASSWORD")
            .ok()
            .filter(|password| !password.is_empty())
            .zip(selected)
            .map(|(password, profile_id)| (profile_id, SecretString::from(password)))
    };

    Ok(StartupProfiles {
        collection,
        unavailable: report.unavailable,
        profiles,
        persisted,
        session_secrets,
        startup_password,
        selected,
        profile_store: store,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::transaction::WorkerDisposition, profile::import_connection_url};

    #[test]
    fn clipboard_failure_feedback_is_not_payload_bearing() {
        let message = clipboard_write_failure_message();

        assert_eq!(message, "Clipboard unavailable");
        assert!(!message.contains("secret"));
    }

    #[test]
    fn catalog_relation_reconciliation_retires_old_and_admits_new_identity() {
        let connection = ConnectionIdentity {
            profile_id: Uuid::new_v4(),
            generation: 1,
        };
        let old = crate::db::catalog::CatalogId::new(
            connection.profile_id,
            crate::db::catalog::CatalogKind::Table,
            ["public", "users", "1"],
        );
        let new = crate::db::catalog::CatalogId::new(
            connection.profile_id,
            crate::db::catalog::CatalogKind::Table,
            ["public", "accounts", "2"],
        );
        let known = StdMutex::new(HashSet::from([(connection, old.clone())]));
        let target = crate::db::catalog::CatalogTarget::objects(
            crate::db::catalog::CatalogId::new(
                connection.profile_id,
                crate::db::catalog::CatalogKind::Schema,
                ["public"],
            ),
            crate::db::catalog::ObjectGroup::Tables,
        )
        .unwrap();
        let targets = StdMutex::new(HashMap::from([((connection, old.clone()), target.clone())]));

        reconcile_catalog_relation(&known, &targets, connection, old.clone(), new.clone());

        assert!(!known.lock().unwrap().contains(&(connection, old)));
        assert!(known.lock().unwrap().contains(&(connection, new.clone())));
        assert_eq!(
            targets.lock().unwrap().get(&(connection, new)),
            Some(&target)
        );
    }

    #[tokio::test]
    async fn quarantine_removal_requires_exact_connection_identity() {
        let profile = import_connection_url("sqlite::memory:", Some("runtime-test"))
            .unwrap()
            .profile;
        let database = DatabaseConnection::connect(&profile, None).await.unwrap();
        let identity = ConnectionIdentity {
            profile_id: profile.id,
            generation: 2,
        };
        let key = ConnectionKey::new(identity, ExecutionTarget::from_profile(&profile));
        let mut active = HashMap::from([(key, ActiveConnection { database })]);

        assert!(
            take_active_connection(
                &mut active,
                ConnectionIdentity {
                    profile_id: profile.id,
                    generation: 1,
                }
            )
            .is_none()
        );
        assert_eq!(active.len(), 1);

        let removed = take_active_connection(
            &mut active,
            ConnectionIdentity {
                profile_id: profile.id,
                generation: 2,
            },
        );
        assert!(removed.is_some());
        assert!(active.is_empty());
        removed.unwrap().database.close().await;
    }

    #[tokio::test]
    async fn finished_manual_worker_entry_is_reaped() {
        let tab_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4();
        let (request_sender, _requests) = tokio::sync::mpsc::unbounded_channel();
        let worker_handle = tokio::spawn(async { WorkerDisposition::Committed });
        tokio::task::yield_now().await;
        assert!(worker_handle.is_finished());
        let mut entries = HashMap::from([(
            tab_id,
            ManualTransactionEntry {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                target: ExecutionTarget {
                    profile_id,
                    database: ":memory:".to_owned(),
                    schema: Some("main".to_owned()),
                },
                transaction_id: Uuid::nil(),
                transaction_generation: 1,
                request_sender,
                worker_handle,
                cancellation_sender: None,
                forced_close_handle: ForcedCloseHandle::new(),
            },
        )]);

        reap_finished_manual_worker(&mut entries, tab_id);

        assert!(!entries.contains_key(&tab_id));
    }
}
