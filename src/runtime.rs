#![allow(clippy::type_complexity)]

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
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
use uuid::Uuid;

use crate::{
    action::{Action, Command},
    app::App,
    cli::{Cli, MouseMode},
    db::{DatabaseConnection, transaction::TransactionRequest},
    input::{
        keymap::{Keymap, map_paste},
        mouse::map_mouse,
    },
    model::{
        profile_manager::{CredentialUpdate, ProfileSubmission},
        workspace::{ConnectionIdentity, QueryStatus},
    },
    persistence::{
        paths::AppPaths,
        profiles::ProfileStore,
        secrets::{
            NativeSecretStore, SecretStore, SecretStoreError, keyring_ref, profile_id_from_ref,
        },
    },
    profile::{ConnectionProfile, import_connection_url},
    security::sanitize_terminal_text,
    terminal::TerminalSession,
    ui::{self, UiState},
};

pub(crate) mod transaction;

use transaction::ForcedCloseHandle;

#[derive(Clone, Debug)]
struct ActiveConnection {
    profile_id: Uuid,
    generation: u64,
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
    transaction_generation: u64,
    request_sender: tokio::sync::mpsc::UnboundedSender<crate::db::transaction::TransactionRequest>,
    worker_handle: JoinHandle<crate::db::transaction::WorkerDisposition>,
    cancellation_sender: Option<tokio::sync::oneshot::Sender<()>>,
    forced_close_handle: Arc<StdMutex<Option<ForcedCloseHandle>>>,
}

pub struct Runtime {
    registry: Arc<Mutex<ProfileRegistry>>,
    profile_store: ProfileStore,
    secret_store: Arc<dyn SecretStore>,
    profile_mutation: Arc<Mutex<()>>,
    event_sender: mpsc::UnboundedSender<Action>,
    connection: Arc<Mutex<Option<ActiveConnection>>>,
    connection_attempts: Arc<StdMutex<ConnectionAttemptTracker>>,
    query_tasks: HashMap<(Uuid, u64), JoinHandle<()>>,
    background_tasks: Vec<JoinHandle<()>>,
    profile_tasks: Vec<JoinHandle<()>>,
    completion_tasks: HashMap<Uuid, JoinHandle<()>>,
    manual_transactions: HashMap<Uuid, ManualTransactionEntry>,
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
            secret_store,
            profile_mutation: Arc::new(Mutex::new(())),
            event_sender,
            connection: Arc::new(Mutex::new(None)),
            connection_attempts: Arc::new(StdMutex::new(ConnectionAttemptTracker::default())),
            query_tasks: HashMap::new(),
            background_tasks: Vec::new(),
            profile_tasks: Vec::new(),
            completion_tasks: HashMap::new(),
            manual_transactions: HashMap::new(),
        }
    }

    pub fn dispatch(&mut self, command: Command) {
        self.query_tasks.retain(|_, task| !task.is_finished());
        self.background_tasks.retain(|task| !task.is_finished());
        self.profile_tasks.retain(|task| !task.is_finished());
        match command {
            Command::TestProfile {
                request_id,
                submission,
            } => self.test_profile(request_id, submission),
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
            } => self.connect(profile_id, generation),
            Command::LoadCatalog {
                profile_id,
                generation,
            } => self.load_catalog(profile_id, generation),
            Command::RunQuery {
                connection,
                tab_id,
                generation,
                sql,
            } => self.run_query(connection, tab_id, generation, sql),
            Command::ManualBegin {
                connection,
                tab_id,
                query_generation,
                transaction_generation,
            } => self.manual_begin(connection, tab_id, query_generation, transaction_generation),
            Command::ManualExecute {
                connection,
                tab_id,
                query_generation,
                transaction_generation,
                sql,
            } => self.manual_execute(
                connection,
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
            Command::PreviewTable {
                connection,
                tab_id,
                generation,
                schema,
                name,
            } => self.preview_table(connection, tab_id, generation, schema, name),
            Command::LoadDdl {
                connection,
                tab_id,
                generation,
                kind,
                schema,
                name,
            } => self.load_ddl(connection, tab_id, generation, kind, schema, name),
            Command::CancelQuery { tab_id, generation } => {
                if let Some(task) = self.query_tasks.remove(&(tab_id, generation)) {
                    task.abort();
                }
            }
            Command::CancelManual {
                tab_id,
                connection,
                transaction_generation,
                ..
            } => {
                if let Some(entry) = self.manual_transactions.get_mut(&tab_id)
                    && entry.connection == connection
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
            Command::PersistWorkspace | Command::Quit => {}
        }
    }

    fn retire_manual_transaction(
        &mut self,
        tab_id: Uuid,
        connection: ConnectionIdentity,
        transaction_generation: u64,
    ) {
        let matches = self.manual_transactions.get(&tab_id).is_some_and(|entry| {
            entry.connection == connection && entry.transaction_generation == transaction_generation
        });
        if matches {
            self.manual_transactions.remove(&tab_id);
        }
    }

    fn manual_entry_matches(
        &self,
        tab_id: Uuid,
        connection: ConnectionIdentity,
        transaction_generation: u64,
    ) -> bool {
        self.manual_transactions.get(&tab_id).is_some_and(|entry| {
            entry.connection == connection && entry.transaction_generation == transaction_generation
        })
    }

    fn test_profile(&mut self, request_id: u64, submission: ProfileSubmission) {
        let registry = Arc::clone(&self.registry);
        let secret_store = Arc::clone(&self.secret_store);
        let sender = self.event_sender.clone();
        self.background_tasks.push(tokio::spawn(async move {
            let ProfileSubmission {
                profile,
                credential,
            } = submission;
            let password =
                match resolve_submission_password(&registry, &secret_store, &profile, credential)
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
                Ok(database) => {
                    let result = database.probe().await;
                    database.close().await;
                    match result {
                        Ok(server) => {
                            let _ =
                                sender.send(Action::ProfileTestSucceeded { request_id, server });
                        }
                        Err(error) => {
                            let _ = sender.send(Action::ProfileTestFailed {
                                request_id,
                                message: sanitize_terminal_text(&error.to_string()),
                            });
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(Action::ProfileTestFailed {
                        request_id,
                        message: sanitize_terminal_text(&error.to_string()),
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
        let sender = self.event_sender.clone();
        self.profile_tasks.push(tokio::spawn(async move {
            let _mutation_guard = mutation.lock().await;
            match save_profile_transaction(registry, profile_store, secret_store, submission).await
            {
                Ok(saved) => {
                    let _ = sender.send(Action::ProfileSaved {
                        request_id,
                        profile: saved.profile,
                        warning: saved.warning,
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
                active.database.close().await;
            }
            let _ = sender.send(Action::DisconnectCompleted {
                connection: expected,
            });
        }));
    }

    fn connect(&mut self, profile_id: Uuid, generation: u64) {
        let registry = Arc::clone(&self.registry);
        let mutation = Arc::clone(&self.profile_mutation);
        let secret_store = Arc::clone(&self.secret_store);
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let attempts = Arc::clone(&self.connection_attempts);
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
            let password = match resolve_profile_password(&registry, &secret_store, &profile).await
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
            match DatabaseConnection::connect(&profile, password.as_ref()).await {
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
                                Some(active.replace(ActiveConnection {
                                    profile_id,
                                    generation,
                                    database:
                                        candidate.take().expect("connection candidate exists"),
                                }))
                            }
                        };
                        drop(active);
                        let Some(previous) = installation else {
                            candidate
                                .take()
                                .expect("connection candidate exists")
                                .close()
                                .await;
                            return;
                        };
                        let _ = sender.send(Action::ConnectionSucceeded {
                            profile_id,
                            generation,
                            server,
                        });
                        drop(mutation_guard);
                        if let Some(previous) = previous {
                            previous.database.close().await;
                        }
                    }
                    Err(error) => {
                        database.close().await;
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

    fn load_catalog(&mut self, profile_id: Uuid, generation: u64) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        self.background_tasks.push(tokio::spawn(async move {
            let database = {
                let guard = connection.lock().await;
                guard
                    .as_ref()
                    .filter(|active| {
                        active.profile_id == profile_id && active.generation == generation
                    })
                    .map(|active| active.database.clone())
            };
            let Some(database) = database else {
                return;
            };
            match database.load_catalog().await {
                Ok(nodes) => {
                    let _ = sender.send(Action::CatalogLoaded {
                        profile_id,
                        generation,
                        nodes,
                    });
                }
                Err(error) => {
                    let _ = sender.send(Action::CatalogFailed {
                        profile_id,
                        generation,
                        message: error.to_string(),
                    });
                }
            }
        }));
    }

    fn run_query(
        &mut self,
        expected: ConnectionIdentity,
        tab_id: Uuid,
        generation: u64,
        sql: String,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let database = active_database(connection, expected).await;
            let Some(database) = database else {
                let _ = sender.send(Action::QueryFailed {
                    tab_id,
                    generation,
                    connection: expected,
                    message: "No active database connection".to_owned(),
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

    fn manual_begin(
        &mut self,
        connection: ConnectionIdentity,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
    ) {
        self.ensure_manual_worker(
            connection,
            tab_id,
            query_generation,
            transaction_generation,
            None,
        );
    }

    fn manual_execute(
        &mut self,
        connection: ConnectionIdentity,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
        sql: String,
    ) {
        let (reply, result) = tokio::sync::oneshot::channel();
        let (cancel, cancel_receiver) = tokio::sync::oneshot::channel();
        self.ensure_manual_worker(
            connection,
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
        if !self.manual_entry_matches(tab_id, connection, transaction_generation) {
            let _ = self.event_sender.send(Action::ManualCommitFailed {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                message: "Stale or missing manual transaction identity".to_owned(),
                unknown: false,
            });
            return;
        }
        let entry = self
            .manual_transactions
            .get(&tab_id)
            .expect("entry was validated");
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
        if !self.manual_entry_matches(tab_id, connection, transaction_generation) {
            let _ = self.event_sender.send(Action::ManualRollbackFailed {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                message: "Stale or missing manual transaction identity".to_owned(),
                unknown: false,
            });
            return;
        }
        let entry = self
            .manual_transactions
            .get(&tab_id)
            .expect("entry was validated");
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
            match request {
                Some((sql, cancel, reply))
                    if entry.connection == connection
                        && entry.transaction_generation == transaction_generation =>
                {
                    let _ = entry.request_sender.send(TransactionRequest::Execute {
                        query_generation,
                        sql,
                        cancel,
                        reply,
                    });
                }
                Some((_, _, reply)) => {
                    let _ = reply.send(Err(crate::db::transaction::TransactionError(
                        "Stale or mismatched manual transaction identity".to_owned(),
                    )));
                }
                None => {}
            }
            return;
        }
        let (proxy, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let forced_close_slot = Arc::new(StdMutex::new(None));
        let worker_forced_close_slot = Arc::clone(&forced_close_slot);
        let database = Arc::clone(&self.connection);
        let sender = self.event_sender.clone();
        let worker_handle = tokio::spawn(async move {
            let Some(database) = active_database(database, connection).await else {
                let _ = sender.send(Action::ManualStartFailed {
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    message: "No active database connection".to_owned(),
                });
                return crate::db::transaction::WorkerDisposition::Quarantine;
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
                requests,
                worker,
                forced_close,
                readiness,
                ..
            } = worker;
            match readiness.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    let _ = sender.send(Action::ManualStartFailed {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                        message: error.to_string(),
                    });
                    return worker
                        .await
                        .unwrap_or(crate::db::transaction::WorkerDisposition::Quarantine);
                }
                Err(_) => {
                    let _ = sender.send(Action::ManualStartFailed {
                        tab_id,
                        query_generation,
                        transaction_generation,
                        connection,
                        message: "Transaction worker readiness was lost".to_owned(),
                    });
                    return worker
                        .await
                        .unwrap_or(crate::db::transaction::WorkerDisposition::Quarantine);
                }
            }
            *worker_forced_close_slot
                .lock()
                .expect("forced-close slot mutex poisoned") = Some(forced_close);
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
                        if disposition.as_ref().is_ok_and(|d| *d == crate::db::transaction::WorkerDisposition::ImplicitlyEnded) {
                            let _ = sender.send(Action::ManualImplicitlyEnded { tab_id, query_generation, transaction_generation, connection });
                        }
                        return disposition.unwrap_or(crate::db::transaction::WorkerDisposition::Quarantine);
                    }
                }
            }
            worker
                .await
                .unwrap_or(crate::db::transaction::WorkerDisposition::Quarantine)
        });
        self.manual_transactions.insert(
            tab_id,
            ManualTransactionEntry {
                connection,
                transaction_generation,
                request_sender: proxy,
                worker_handle,
                cancellation_sender: None,
                forced_close_handle: forced_close_slot,
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

    fn preview_table(
        &mut self,
        expected: ConnectionIdentity,
        tab_id: Uuid,
        generation: u64,
        schema: String,
        name: String,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database(connection, expected).await else {
                let _ = sender.send(Action::QueryFailed {
                    tab_id,
                    generation,
                    connection: expected,
                    message: "No active database connection".to_owned(),
                });
                return;
            };
            let sql = format!(
                "SELECT * FROM {}.{} LIMIT 500",
                database.quote_identifier(&schema),
                database.quote_identifier(&name)
            );
            match database.execute(&sql).await {
                Ok(outcome) => {
                    let _ = sender.send(Action::PreviewFinished {
                        tab_id,
                        generation,
                        sql,
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

    fn load_ddl(
        &mut self,
        expected: ConnectionIdentity,
        tab_id: Uuid,
        generation: u64,
        kind: crate::db::catalog::CatalogKind,
        schema: String,
        name: String,
    ) {
        let sender = self.event_sender.clone();
        let connection = Arc::clone(&self.connection);
        let task = tokio::spawn(async move {
            let Some(database) = active_database(connection, expected).await else {
                let _ = sender.send(Action::QueryFailed {
                    tab_id,
                    generation,
                    connection: expected,
                    message: "No active database connection".to_owned(),
                });
                return;
            };
            match database.object_ddl(kind, &schema, &name).await {
                Ok(Some(ddl)) => {
                    let _ = sender.send(Action::DdlLoaded {
                        tab_id,
                        generation,
                        ddl,
                    });
                }
                Ok(None) => {
                    let _ = sender.send(Action::QueryFailed {
                        tab_id,
                        generation,
                        connection: expected,
                        message: "DDL is not available for this object type".to_owned(),
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

    pub async fn shutdown(mut self) {
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
            let forced_close = entry
                .forced_close_handle
                .lock()
                .expect("forced-close slot mutex poisoned")
                .clone();
            let mut worker = entry.worker_handle;
            if timeout(Duration::from_secs(2), &mut worker).await.is_err() {
                worker.abort();
                let _ = worker.await;
                if let Some(forced_close) = forced_close {
                    let _ = timeout(Duration::from_secs(2), async {
                        while !forced_close.completed() {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await;
                }
            }
        }
        if let Some(connection) = self.connection.lock().await.take() {
            connection.database.close().await;
        }
    }
}

struct SavedProfile {
    profile: ConnectionProfile,
    warning: Option<String>,
}

async fn save_profile_transaction(
    registry: Arc<Mutex<ProfileRegistry>>,
    profile_store: ProfileStore,
    secret_store: Arc<dyn SecretStore>,
    submission: ProfileSubmission,
) -> Result<SavedProfile, String> {
    let snapshot = registry.lock().await.clone();
    let ProfileSubmission {
        mut profile,
        credential,
    } = submission;
    let profile_id = profile.id;
    let old_profile = snapshot.profiles.get(&profile_id).cloned();
    let mut next = snapshot;
    let mut previous_secret = None;
    let mut warning = None;

    match credential {
        CredentialUpdate::Preserve => {
            profile.secret_ref = if let Some(old_profile) = old_profile.as_ref() {
                validate_secret_reference(old_profile)?;
                old_profile.secret_ref.clone()
            } else {
                None
            };
        }
        CredentialUpdate::Session(password) => {
            if let Some(old_profile) = old_profile.as_ref()
                && old_profile.secret_ref.is_some()
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
            profile.secret_ref = None;
            next.session_secrets.insert(profile_id, password);
        }
        CredentialUpdate::Remember(password) => {
            if let Some(old_profile) = old_profile.as_ref()
                && old_profile.secret_ref.is_some()
            {
                validate_secret_reference(old_profile)?;
            }
            match remember_secret(&secret_store, profile_id, &password).await? {
                RememberResult::Stored { previous } => {
                    previous_secret = Some(previous);
                    profile.secret_ref = Some(keyring_ref(profile_id));
                }
                RememberResult::SessionOnly => {
                    profile.secret_ref = None;
                    warning = Some(
                        if old_profile
                            .as_ref()
                            .is_some_and(|profile| profile.secret_ref.is_some())
                        {
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
        CredentialUpdate::Forget => {
            if let Some(old_profile) = old_profile.as_ref()
                && old_profile.secret_ref.is_some()
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
            profile.secret_ref = None;
            next.session_secrets.remove(&profile_id);
        }
    }

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
    Ok(SavedProfile { profile, warning })
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
    if profile.secret_ref.is_some() {
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
    profile: &ConnectionProfile,
    credential: CredentialUpdate,
) -> Result<Option<SecretString>, String> {
    match credential {
        CredentialUpdate::Session(password) | CredentialUpdate::Remember(password) => {
            Ok(Some(password))
        }
        CredentialUpdate::Forget => Ok(None),
        CredentialUpdate::Preserve => {
            resolve_profile_password(registry, secret_store, profile).await
        }
    }
}

async fn resolve_profile_password(
    registry: &Arc<Mutex<ProfileRegistry>>,
    secret_store: &Arc<dyn SecretStore>,
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
    if profile.secret_ref.is_none() {
        return Ok(None);
    }
    validate_secret_reference(profile)?;
    match read_secret(secret_store, profile.id).await {
        Ok(Some(password)) => Ok(Some(password)),
        Ok(None) => Err("Stored password is missing; enter a password to continue".to_owned()),
        Err(SecretStoreError::Locked | SecretStoreError::Unavailable) => {
            Err("Stored password is unavailable; enter a password to continue".to_owned())
        }
        Err(error) => Err(secret_error("Unable to read the stored password", error)),
    }
}

fn validate_secret_reference(profile: &ConnectionProfile) -> Result<(), String> {
    let Some(reference) = profile.secret_ref.as_deref() else {
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

pub async fn run_tui(cli: Cli) -> Result<()> {
    let startup = load_startup_profiles(&cli)?;
    let mut app = App::with_confirmation_policy(startup.profiles.clone(), cli.confirm_execution);
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
    let mut terminal = TerminalSession::enter(cli.mouse != MouseMode::Off)
        .context("failed to initialize terminal")?;
    let mut terminal_events = EventStream::new();
    let mut keymap = Keymap::default();
    let mut ui_state = UiState::default();
    let mut ticker = interval(Duration::from_millis(33));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let result: Result<()> = async {
        apply_action(
            &mut app,
            &mut runtime,
            startup
                .selected
                .map_or(Action::OpenProfileManager, Action::RequestConnect),
        );
        terminal.draw(|frame| ui::render_with_state(frame, &app, &mut ui_state))?;
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
                    redraw = ui_state.effects.is_active()
                        || app.tabs.iter().any(|tab| tab.query_status == QueryStatus::Running);
                }
            }

            if redraw && !app.should_quit {
                terminal.draw(|frame| ui::render_with_state(frame, &app, &mut ui_state))?;
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

fn apply_action(app: &mut App, runtime: &mut Runtime, action: Action) {
    match &action {
        Action::ManualCommitted {
            tab_id,
            transaction_generation,
            connection,
            ..
        }
        | Action::ManualRolledBack {
            tab_id,
            transaction_generation,
            connection,
            ..
        }
        | Action::ManualImplicitlyEnded {
            tab_id,
            transaction_generation,
            connection,
            ..
        } => runtime.retire_manual_transaction(*tab_id, *connection, *transaction_generation),
        _ => {}
    }
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
