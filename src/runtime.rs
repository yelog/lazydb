#![allow(clippy::type_complexity)]

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use uuid::Uuid;

use crate::{
    action::{Action, Command},
    app::App,
    cli::{Cli, MouseMode},
    db::{
        DatabaseConnection,
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
        workspace::{ConnectionIdentity, QueryStatus},
    },
    persistence::{
        local_credentials::LocalCredentialStore,
        paths::AppPaths,
        profiles::ProfileStore,
        secrets::{
            NativeSecretStore, SecretStore, SecretStoreAvailability, SecretStoreError, keyring_ref,
            profile_id_from_ref,
        },
        workspace::WorkspaceStore,
    },
    profile::{ConnectionProfile, CredentialPolicy, import_connection_url},
    security::sanitize_terminal_text,
    terminal::TerminalSession,
    ui::{self, UiState},
};
use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use secrecy::SecretString;
use tokio::{
    sync::{Mutex, mpsc},
    task::{self, JoinHandle},
    time::sleep,
    time::{MissedTickBehavior, interval, timeout},
};

pub(crate) mod transaction;

use transaction::ForcedCloseHandle;

#[derive(Clone, Debug)]
struct ActiveConnection {
    profile_id: Uuid,
    generation: u64,
    target: ExecutionTarget,
    database: DatabaseConnection,
}

#[derive(Default)]
struct ConnectionAttemptTracker {
    latest: Option<ConnectionIdentity>,
    cancelled: Option<ConnectionIdentity>,
}

#[derive(Clone)]
struct ProfileRegistry {
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
    transaction_generation: u64,
    request_sender: tokio::sync::mpsc::UnboundedSender<crate::db::transaction::TransactionRequest>,
    worker_handle: JoinHandle<crate::db::transaction::WorkerDisposition>,
    cancellation_sender: Option<tokio::sync::oneshot::Sender<()>>,
    forced_close_handle: ForcedCloseHandle,
}

pub struct Runtime {
    registry: Arc<Mutex<ProfileRegistry>>,
    profile_store: ProfileStore,
    workspace_store: Option<WorkspaceStore>,
    workspace_mutation: Arc<Mutex<()>>,
    secret_store: Arc<dyn SecretStore>,
    local_credential_store: LocalCredentialStore,
    profile_mutation: Arc<Mutex<()>>,
    event_sender: mpsc::UnboundedSender<Action>,
    connection: Arc<Mutex<Option<ActiveConnection>>>,
    connection_attempts: Arc<StdMutex<ConnectionAttemptTracker>>,
    query_tasks: HashMap<(Uuid, u64), JoinHandle<()>>,
    relation_tasks: HashMap<crate::model::relation::RelationRequest, JoinHandle<()>>,
    known_relations: Arc<StdMutex<HashSet<(ConnectionIdentity, crate::db::catalog::CatalogId)>>>,
    background_tasks: Vec<JoinHandle<()>>,
    profile_tasks: Vec<JoinHandle<()>>,
    completion_tasks: HashMap<Uuid, JoinHandle<()>>,
    manual_transactions: HashMap<Uuid, ManualTransactionEntry>,
    relation_transactions: HashMap<Uuid, ManualTransactionEntry>,
    relation_mutation_blocked: Arc<StdMutex<HashSet<(Uuid, ConnectionIdentity)>>>,
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
            secret_store,
            local_credential_store,
            profile_mutation: Arc::new(Mutex::new(())),
            event_sender,
            connection: Arc::new(Mutex::new(None)),
            connection_attempts: Arc::new(StdMutex::new(ConnectionAttemptTracker::default())),
            query_tasks: HashMap::new(),
            relation_tasks: HashMap::new(),
            known_relations: Arc::new(StdMutex::new(HashSet::new())),
            background_tasks: Vec::new(),
            profile_tasks: Vec::new(),
            completion_tasks: HashMap::new(),
            manual_transactions: HashMap::new(),
            relation_transactions: HashMap::new(),
            relation_mutation_blocked: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    pub fn set_workspace_store(&mut self, store: WorkspaceStore) {
        self.workspace_store = Some(store);
    }

    pub fn dispatch(&mut self, command: Command) {
        self.query_tasks.retain(|_, task| !task.is_finished());
        self.relation_tasks.retain(|_, task| !task.is_finished());
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
            Command::Disconnect { connection } => self.disconnect(connection),
            Command::Connect {
                profile_id,
                generation,
                target,
            } => self.connect(profile_id, generation, target),
            Command::LoadCatalogPage(request) => self.load_catalog_page(request),
            Command::LoadRelationPreview(request) | Command::LoadRelationStructure(request) => {
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
            Command::PersistWorkspace(snapshot) => {
                if let Some(store) = self.workspace_store.clone() {
                    let mutation = Arc::clone(&self.workspace_mutation);
                    self.background_tasks.push(tokio::spawn(async move {
                        let _guard = mutation.lock().await;
                        let _ = tokio::task::spawn_blocking(move || store.save(&snapshot)).await;
                    }));
                }
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

    fn disconnect(&mut self, expected: ConnectionIdentity) {
        let connection = Arc::clone(&self.connection);
        let known_relations = Arc::clone(&self.known_relations);
        let mutation = Arc::clone(&self.profile_mutation);
        let attempts = Arc::clone(&self.connection_attempts);
        let sender = self.event_sender.clone();
        {
            let mut attempts = attempts.lock().expect("connection attempt mutex poisoned");
            if attempts.latest == Some(expected) {
                attempts.cancelled = Some(expected);
            }
        }
        self.background_tasks.push(tokio::spawn(async move {
            let _mutation_guard = mutation.lock().await;
            let active = {
                let mut guard = connection.lock().await;
                if guard.as_ref().is_some_and(|active| {
                    active.profile_id == expected.profile_id
                        && active.generation == expected.generation
                }) {
                    guard.take()
                } else {
                    None
                }
            };
            if let Some(active) = active {
                if let Ok(mut known) = known_relations.lock() {
                    known.clear();
                }
                active.database.close().await;
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
        let expected = ConnectionIdentity {
            profile_id,
            generation,
        };
        {
            let mut attempts = attempts.lock().expect("connection attempt mutex poisoned");
            if attempts
                .latest
                .is_some_and(|latest| generation <= latest.generation)
            {
                return;
            }
            attempts.latest = Some(expected);
            attempts.cancelled = None;
        }
        self.background_tasks.push(tokio::spawn(async move {
            let mutation_guard = mutation.lock().await;
            let profile = {
                let registry = registry.lock().await;
                registry.profiles.get(&profile_id).cloned().map(|profile| {
                    let revision = registry.revisions.get(&profile_id).copied().unwrap_or(0);
                    (profile, revision)
                })
            };
            let Some((profile, profile_revision)) = profile else {
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
                    .as_ref()
                    .filter(|active| active.profile_id == profile_id)
                    .map(|active| active.database.clone())
            } else {
                None
            };
            let reused_sqlite = reusable_sqlite.is_some();
            let candidate = match reusable_sqlite {
                Some(database) => Ok(database),
                None => {
                    DatabaseConnection::connect_target(&profile, password.as_ref(), &target).await
                }
            };
            match candidate {
                Ok(database) => match database.probe().await {
                    Ok(server) => {
                        let mutation_guard = mutation.lock().await;
                        if !profile_revision_is_current(&registry, &profile, profile_revision).await
                        {
                            database.close().await;
                            return;
                        }
                        let mut active = connection.lock().await;
                        let mut candidate = Some(database);
                        let installation = {
                            let attempt_guard =
                                attempts.lock().expect("connection attempt mutex poisoned");
                            if attempt_guard.latest != Some(expected)
                                || attempt_guard.cancelled == Some(expected)
                            {
                                None
                            } else {
                                if let Ok(mut known) = known_relations.lock() {
                                    known.clear();
                                }
                                Some(active.replace(ActiveConnection {
                                    profile_id,
                                    generation,
                                    target: target.clone(),
                                    database:
                                        candidate.take().expect("connection candidate exists"),
                                }))
                            }
                        };
                        drop(active);
                        let Some(previous) = installation else {
                            let candidate = candidate.take().expect("connection candidate exists");
                            if !reused_sqlite {
                                candidate.close().await;
                            }
                            return;
                        };
                        let _ = sender.send(Action::ConnectionSucceeded {
                            profile_id,
                            generation,
                            server,
                        });
                        drop(mutation_guard);
                        if let Some(previous) = previous
                            && !reused_sqlite
                        {
                            previous.database.close().await;
                        }
                    }
                    Err(error) => {
                        if !reused_sqlite {
                            database.close().await;
                        }
                        let _mutation_guard = mutation.lock().await;
                        if profile_revision_is_current(&registry, &profile, profile_revision).await
                            && connection_attempt_is_current(&attempts, expected)
                        {
                            let _ = sender.send(Action::ConnectionFailed {
                                profile_id,
                                generation,
                                message: error.to_string(),
                            });
                        }
                    }
                },
                Err(error) => {
                    let _mutation_guard = mutation.lock().await;
                    if profile_revision_is_current(&registry, &profile, profile_revision).await
                        && connection_attempt_is_current(&attempts, expected)
                    {
                        let _ = sender.send(Action::ConnectionFailed {
                            profile_id,
                            generation,
                            message: error.to_string(),
                        });
                    }
                }
            }
        }));
    }

    fn load_catalog_page(&mut self, request: crate::db::catalog::CatalogRequest) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let known_relations = Arc::clone(&self.known_relations);
        self.background_tasks.push(tokio::spawn(async move {
            let key = request.key.clone();
            let database = {
                let guard = connection.lock().await;
                guard
                    .as_ref()
                    .filter(|active| {
                        active.profile_id == key.connection.profile_id
                            && active.generation == key.connection.generation
                    })
                    .map(|active| active.database.clone())
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
                        let active =
                            active_database(Arc::clone(&connection), request.key.connection)
                                .await
                                .is_some();
                        if !active {
                            return;
                        }
                        if let Ok(mut known) = known_relations.lock() {
                            known.extend(
                                page.entries
                                    .iter()
                                    .filter(|entry| entry.kind.is_relation())
                                    .map(|entry| (request.key.connection, entry.id.clone())),
                            );
                        }
                        let _ = sender.send(Action::CatalogPageLoaded(page));
                    }
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogPageFailed {
                        key,
                        category: error.category,
                        message: error.to_string(),
                    });
                }
            }
        }));
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
        let active = self.connection.try_lock().ok().and_then(|guard| {
            guard.as_ref().map(|active| ConnectionIdentity {
                profile_id: active.profile_id,
                generation: active.generation,
            })
        }) == Some(request.connection);
        if !active {
            let _ = self.event_sender.send(Action::RelationFailed {
                request,
                message: "No active database connection".to_owned(),
            });
            return;
        }
        if !self.known_relations.lock().is_ok_and(|known| {
            known.contains(&(request.connection, request.relation.object_id.clone()))
        }) {
            let _ = self.event_sender.send(Action::RelationFailed {
                request,
                message: "relation is not present in the active catalog snapshot".to_owned(),
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
                    .preview_relation(&task_request.relation.object_id, &task_request.options)
                    .await
                    .map(crate::model::relation::RelationSnapshot::Preview),
                crate::model::relation::RelationRequestKind::Structure => database
                    .relation_structure(&task_request.relation.object_id)
                    .await
                    .map(|snapshot| {
                        crate::model::relation::RelationSnapshot::Structure(Box::new(snapshot))
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
        let task = tokio::spawn(async move {
            let database = active_database_for_target(connection, expected, &target).await;
            let Some(database) = database else {
                let _ = sender.send(Action::QueryFailed {
                    tab_id,
                    generation,
                    connection: expected,
                    message: "Active connection does not match the execution target".to_owned(),
                });
                return;
            };
            match database.execute(&sql).await {
                Ok(outcome) => {
                    let _ = sender.send(Action::QueryFinished {
                        tab_id,
                        generation,
                        connection: expected,
                        outcome,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::QueryFailed {
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
            match database.execute(&sql).await {
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
                        message: error.0,
                    });
                }
                Err(_) => {
                    if let Ok(mut blocked) = blocked.lock() {
                        blocked.insert((tab_id, request.connection));
                    }
                    let _ = sender.send(Action::RelationMutationFailed {
                        request,
                        message: "Relation mutation acknowledgement was lost".into(),
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
                });
                return crate::db::transaction::WorkerDisposition::Quarantine;
            };
            let invalidates = matches!(database, DatabaseConnection::Sqlite(_));
            let worker = match database.start_transaction_worker().await {
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
                transaction_generation,
                request_sender: proxy.clone(),
                worker_handle,
                cancellation_sender: None,
                forced_close_handle: ForcedCloseHandle::new(),
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
        self.ensure_manual_worker(
            connection,
            target,
            tab_id,
            query_generation,
            transaction_generation,
            Some((sql, cancel_receiver, reply)),
        );
        if let Some(entry) = self.manual_transactions.get_mut(&tab_id) {
            entry.cancellation_sender = Some(cancel);
            let sender = self.event_sender.clone();
            self.query_tasks.insert(
                (tab_id, query_generation),
                tokio::spawn(async move {
                    match result.await {
                        Ok(Ok(outcome)) => {
                            let _ = sender.send(Action::ManualQueryFinished {
                                tab_id,
                                query_generation,
                                transaction_generation,
                                connection,
                                outcome,
                            });
                        }
                        Ok(Err(error)) => {
                            let _ = sender.send(Action::ManualQueryFailed {
                                tab_id,
                                query_generation,
                                transaction_generation,
                                connection,
                                message: error.0,
                            });
                        }
                        Err(_) => {
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
        self.background_tasks.push(tokio::spawn(async move {
            match result.await {
                Ok(Ok(())) => {
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
        self.background_tasks.push(tokio::spawn(async move {
            match result.await {
                Ok(Ok(())) => {
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
                DatabaseConnection::Postgres(_) | DatabaseConnection::MySql(_) => false,
            };
            let worker = match database.start_transaction_worker().await {
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
                transaction_generation,
                request_sender: proxy,
                worker_handle,
                cancellation_sender: None,
                forced_close_handle: ForcedCloseHandle::new(),
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
        for (_, task) in self.relation_tasks.drain() {
            task.abort();
            let _ = task.await;
        }
        for (_, task) in self.query_tasks.drain() {
            task.abort();
            let _ = task.await;
        }
        for task in self.background_tasks.drain(..) {
            task.abort();
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
        if let Some(connection) = self.connection.lock().await.take() {
            connection.database.close().await;
        }
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

    if !next.profiles.contains_key(&profile_id) {
        next.order.push(profile_id);
    }
    next.profiles.insert(profile_id, profile.clone());
    *next.revisions.entry(profile_id).or_default() += 1;
    next.persisted.insert(profile_id);
    let persisted_profiles = next.ordered_persisted_profiles();
    if let Err(primary) = save_profiles(profile_store, persisted_profiles).await {
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
    connection: Arc<Mutex<Option<ActiveConnection>>>,
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
        let persisted_profiles = next.ordered_persisted_profiles();
        if let Err(primary) = save_profiles(profile_store, persisted_profiles).await {
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
        .as_ref()
        .filter(|active| active.profile_id == profile_id)
        .map(|active| ConnectionIdentity {
            profile_id: active.profile_id,
            generation: active.generation,
        });
    *registry.lock().await = next;
    Ok(active_connection)
}

impl ProfileRegistry {
    fn ordered_persisted_profiles(&self) -> Vec<ConnectionProfile> {
        self.order
            .iter()
            .filter(|profile_id| self.persisted.contains(profile_id))
            .filter_map(|profile_id| self.profiles.get(profile_id).cloned())
            .collect()
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
    match &profile.credential_policy {
        CredentialPolicy::None => Ok(None),
        CredentialPolicy::Prompt => Err("Enter a password to continue".to_owned()),
        CredentialPolicy::LocalEncrypted(credential) => {
            let password = local_credential_store
                .decrypt(profile.id, credential)
                .map_err(|error| {
                    sanitize_terminal_text(&format!(
                        "Unable to decrypt the local password: {error}"
                    ))
                })?;
            registry
                .lock()
                .await
                .session_secrets
                .insert(profile.id, password.clone());
            Ok(Some(password))
        }
        CredentialPolicy::System(_) | CredentialPolicy::Keyring(_) => {
            validate_secret_reference(profile)?;
            match read_secret(secret_store, profile.id).await {
                Ok(Some(password)) => {
                    registry
                        .lock()
                        .await
                        .session_secrets
                        .insert(profile.id, password.clone());
                    Ok(Some(password))
                }
                Ok(None) => {
                    Err("Stored password is missing; enter a password to continue".to_owned())
                }
                Err(SecretStoreError::Locked | SecretStoreError::Unavailable) => {
                    Err("Stored password is unavailable; enter a password to continue".to_owned())
                }
                Err(error) => Err(secret_error("Unable to read the stored password", error)),
            }
        }
    }
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
    profiles: Vec<ConnectionProfile>,
) -> Result<(), String> {
    task::spawn_blocking(move || profile_store.save(&profiles))
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
    attempts: &StdMutex<ConnectionAttemptTracker>,
    expected: ConnectionIdentity,
) -> bool {
    let attempts = attempts.lock().expect("connection attempt mutex poisoned");
    attempts.latest == Some(expected) && attempts.cancelled != Some(expected)
}

async fn active_database(
    connection: Arc<Mutex<Option<ActiveConnection>>>,
    expected: ConnectionIdentity,
) -> Option<DatabaseConnection> {
    connection
        .lock()
        .await
        .as_ref()
        .filter(|active| {
            active.profile_id == expected.profile_id && active.generation == expected.generation
        })
        .map(|active| active.database.clone())
}

async fn active_database_for_target(
    connection: Arc<Mutex<Option<ActiveConnection>>>,
    expected: ConnectionIdentity,
    target: &ExecutionTarget,
) -> Option<DatabaseConnection> {
    connection
        .lock()
        .await
        .as_ref()
        .filter(|active| {
            active.profile_id == expected.profile_id
                && active.generation == expected.generation
                && &active.target == target
        })
        .map(|active| active.database.clone())
}

fn take_active_connection(
    active: &mut Option<ActiveConnection>,
    expected: ConnectionIdentity,
) -> Option<ActiveConnection> {
    if active.as_ref().is_some_and(|active| {
        active.profile_id == expected.profile_id && active.generation == expected.generation
    }) {
        active.take()
    } else {
        None
    }
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
    connection: Arc<Mutex<Option<ActiveConnection>>>,
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

pub async fn run_tui(cli: Cli) -> Result<()> {
    let startup = load_startup_profiles(&cli)?;
    let paths = AppPaths::discover()?;
    let workspace_store = WorkspaceStore::new(paths.workspace_file(), paths.workspace_sql_dir());
    let workspace = workspace_store.load().context("failed to load workspace")?;
    let mut app = App::with_startup_profiles_and_confirmation_policy(
        startup.profiles.clone(),
        startup.persisted.clone(),
        cli.confirm_execution,
    );
    if let Some(workspace) = workspace {
        app.restore_workspace(workspace, startup.selected);
    }
    app.focus = crate::model::workspace::Focus::Explorer;
    let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        startup.profiles,
        startup.persisted,
        startup.session_secrets,
        startup.startup_password,
        startup.profile_store,
        Arc::new(NativeSecretStore),
        event_sender,
    );
    runtime.set_workspace_store(workspace_store);
    let mut terminal = TerminalSession::enter(cli.mouse != MouseMode::Off)
        .context("failed to initialize terminal")?;
    let icons = crate::ui::icons::IconSet::new(cli.icons);
    let mut terminal_events = EventStream::new();
    let mut keymap = Keymap::default();
    let mut ui_state = UiState::default();
    let mut ticker = interval(Duration::from_millis(33));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let result: Result<()> = async {
        apply_startup_action_with_runtime(&mut app, &mut runtime, startup.selected);
        runtime.dispatch(Command::CheckSecretStoreAvailability);
        terminal
            .draw(|frame| ui::render_with_state_using_icons(frame, &app, &mut ui_state, icons))?;
        if let Some(style) = ui_state.cursor_style {
            terminal.set_cursor_style(style)?;
        }
        sync_editor_viewport(&mut app, &mut runtime, &ui_state);

        while !app.should_quit {
            let mut redraw = false;
            tokio::select! {
                terminal_event = terminal_events.next() => {
                    let Some(terminal_event) = terminal_event else { break; };
                    match terminal_event.context("terminal input failed")? {
                        Event::Key(key) => {
                            if let Some(action) = keymap.map(key, &app) {
                                apply_action(&mut app, &mut runtime, action);
                                redraw = true;
                            }
                        }
                        Event::Mouse(mouse) => {
                            keymap.clear_pending();
                            if let Some(action) = map_mouse(mouse, &ui_state, &app) {
                                apply_action(&mut app, &mut runtime, action);
                                redraw = true;
                            }
                        }
                        Event::Paste(value) => {
                            keymap.clear_pending();
                            let actions = map_paste(value, &app);
                            if !actions.is_empty() {
                                for action in actions {
                                    apply_action(&mut app, &mut runtime, action);
                                }
                                redraw = true;
                            }
                        }
                        Event::Resize(_, _) | Event::FocusGained | Event::FocusLost => {
                            keymap.clear_pending();
                            redraw = true;
                        }
                    }
                }
                Some(action) = event_receiver.recv() => {
                    apply_action(&mut app, &mut runtime, action);
                    redraw = true;
                }
                _ = ticker.tick() => {
                    redraw = app.tabs.iter().any(|tab| {
                        tab.as_console()
                            .is_some_and(|tab| tab.query_status == QueryStatus::Running)
                    });
                }
            }

            if redraw && !app.should_quit {
                terminal.draw(|frame| {
                    ui::render_with_state_using_icons(frame, &app, &mut ui_state, icons)
                })?;
                if let Some(style) = ui_state.cursor_style {
                    terminal.set_cursor_style(style)?;
                }
                sync_editor_viewport(&mut app, &mut runtime, &ui_state);
            }
        }

        Ok(())
    }
    .await;

    runtime.shutdown().await;
    result
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
    for command in app.update(action) {
        runtime.dispatch(command);
    }
}

fn sync_editor_viewport(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    let Some(viewport) = state.editor_viewport else {
        return;
    };
    if app.active_editor_viewport().ok() != Some(viewport) {
        apply_action(app, runtime, Action::EditorViewportChanged(viewport));
    }
}

pub struct StartupProfiles {
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
    let mut profiles = store.load().context("failed to load connection profiles")?;
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
    let selected = selected.or_else(|| profiles.first().map(|profile| profile.id));

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

    #[tokio::test]
    async fn quarantine_removal_requires_exact_connection_identity() {
        let profile = import_connection_url("sqlite::memory:", Some("runtime-test"))
            .unwrap()
            .profile;
        let database = DatabaseConnection::connect(&profile, None).await.unwrap();
        let mut active = Some(ActiveConnection {
            profile_id: profile.id,
            generation: 2,
            target: ExecutionTarget::from_profile(&profile),
            database,
        });

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
        assert!(active.is_some());

        let removed = take_active_connection(
            &mut active,
            ConnectionIdentity {
                profile_id: profile.id,
                generation: 2,
            },
        );
        assert!(removed.is_some());
        assert!(active.is_none());
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
