use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{Runtime, resolve_profile_password};
use crate::{
    action::Action,
    agent::policy::WritePolicy,
    data_ops::{DataJobPlan, DataJobService, DataJobSpec, DataJobStore},
    model::data_ops::DataOperationAction,
    persistence::{credentials::CredentialResolver, paths::AppPaths, profiles::ProfileStore},
    security::sanitize_terminal_text,
};

pub(super) struct DataOperationsRuntime {
    store: DataJobStore,
    tasks: HashMap<Uuid, (CancellationToken, JoinHandle<()>)>,
    prepared: Arc<Mutex<HashMap<Uuid, (Uuid, Arc<DataJobService>)>>>,
    policy: WritePolicy,
    project_root: Option<PathBuf>,
}

impl DataOperationsRuntime {
    pub(super) fn new(profiles: &ProfileStore) -> Self {
        let directory = AppPaths::discover()
            .ok()
            .filter(|paths| paths.profiles_file() == profiles.path())
            .map(|paths| paths.state_dir)
            .or_else(|| profiles.path().parent().map(std::path::Path::to_path_buf))
            .unwrap_or_default()
            .join("data-jobs");
        Self {
            store: DataJobStore::new(directory),
            tasks: HashMap::new(),
            prepared: Arc::new(Mutex::new(HashMap::new())),
            policy: WritePolicy::All,
            project_root: crate::project::ProjectContext::resolve_current()
                .ok()
                .map(|project| project.root),
        }
    }
}

impl Runtime {
    pub fn set_data_job_store(&mut self, store: DataJobStore) {
        self.data_operations.store = store;
    }

    pub fn set_data_operation_write_policy(&mut self, policy: WritePolicy) {
        self.data_operations.policy = policy;
    }

    pub fn set_data_operation_project_root(&mut self, root: PathBuf) {
        self.data_operations.project_root = Some(root);
    }

    fn data_policy(&self, policy: WritePolicy) -> WritePolicy {
        match (policy, self.data_operations.policy) {
            (WritePolicy::Deny, _) | (_, WritePolicy::Deny) => WritePolicy::Deny,
            (WritePolicy::NonProduction, _) | (_, WritePolicy::NonProduction) => {
                WritePolicy::NonProduction
            }
            _ => WritePolicy::All,
        }
    }

    pub(super) fn plan_data_operation(
        &mut self,
        request_id: Uuid,
        spec: DataJobSpec,
        policy: WritePolicy,
    ) {
        self.data_operations
            .tasks
            .retain(|_, (_, task)| !task.is_finished());
        let policy = self.data_policy(policy);
        let registry = self.registry.clone();
        let secrets = self.secret_store.clone();
        let local = self.local_credential_store.clone();
        let connector = self.connector.clone();
        let store = self.data_operations.store.clone();
        let prepared = self.data_operations.prepared.clone();
        let sender = self.event_sender.clone();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let task = tokio::spawn(async move {
            let result = async {
                let snapshot = registry.lock().await.clone();
                let profiles = snapshot
                    .order
                    .iter()
                    .filter_map(|id| snapshot.profiles.get(id).cloned())
                    .collect::<Vec<_>>();
                let credential_registry = Arc::new(Mutex::new(snapshot));
                let mut passwords = HashMap::new();
                for endpoint in spec.source.iter().chain(spec.destination.iter()) {
                    if passwords.contains_key(&endpoint.profile_id) {
                        continue;
                    }
                    let profile = profiles
                        .iter()
                        .find(|profile| profile.id == endpoint.profile_id)
                        .ok_or_else(|| {
                            anyhow::anyhow!("A selected profile is no longer available")
                        })?;
                    if let Some(password) =
                        resolve_profile_password(&credential_registry, &secrets, &local, profile)
                            .await
                            .map_err(anyhow::Error::msg)?
                    {
                        passwords.insert(profile.id, password);
                    }
                }
                let credentials = Arc::new(CredentialResolver::new(secrets, local));
                let service = Arc::new(
                    DataJobService::new(profiles, credentials, connector, store)
                        .with_passwords(passwords),
                );
                let plan = Box::pin(service.plan(spec, policy, task_cancellation.clone())).await?;
                if task_cancellation.is_cancelled() {
                    anyhow::bail!("Planning was cancelled");
                }
                prepared.lock().await.insert(plan.id, (request_id, service));
                Ok::<_, anyhow::Error>(plan)
            }
            .await
            .map(Box::new)
            .map_err(|error| sanitize_terminal_text(&error.to_string()));
            let _ = sender.send(Action::DataOperation(DataOperationAction::PlanReady {
                request_id,
                result,
            }));
        });
        self.data_operations
            .tasks
            .insert(request_id, (cancellation, task));
    }

    pub(super) fn run_data_operation(
        &mut self,
        request_id: Uuid,
        plan: DataJobPlan,
        policy: WritePolicy,
    ) {
        self.data_operations
            .tasks
            .retain(|_, (_, task)| !task.is_finished());
        let policy = self.data_policy(policy);
        let prepared = self.data_operations.prepared.clone();
        let registry = self.registry.clone();
        let sender = self.event_sender.clone();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let job_id = plan.id;
        let task = tokio::spawn(async move {
            let result = async {
                let (_, service) =
                    prepared.lock().await.remove(&plan.id).ok_or_else(|| {
                        anyhow::anyhow!("This plan has expired; review a new draft")
                    })?;
                {
                    let registry = registry.lock().await;
                    for endpoint in plan.spec.source.iter().chain(plan.spec.destination.iter()) {
                        let current = registry.profiles.get(&endpoint.profile_id)
                            .ok_or_else(|| anyhow::anyhow!(
                                "A reviewed profile is no longer available; review a new draft"
                            ))?;
                        let reviewed = service.profiles().iter()
                            .find(|profile| profile.id == endpoint.profile_id);
                        if reviewed != Some(current) {
                            anyhow::bail!(
                                "A source or target profile changed after review; review a new draft"
                            );
                        }
                    }
                }
                if plan.spec.operation.writes_database()
                    && let Some(destination) = &plan.spec.destination
                {
                    let registry = registry.lock().await;
                    let profile =
                        registry
                            .profiles
                            .get(&destination.profile_id)
                            .ok_or_else(|| {
                                anyhow::anyhow!("The target profile is no longer available")
                            })?;
                    if let crate::agent::policy::WriteCapability::Denied(error) =
                        crate::agent::policy::write_capability(profile, policy)
                    {
                        anyhow::bail!("{error}");
                    }
                }
                let updates = sender.clone();
                let on_update = Arc::new(move |record| {
                    let _ = updates.send(Action::DataOperation(DataOperationAction::JobUpdated {
                        request_id,
                        record: Box::new(record),
                    }));
                });
                Box::pin(service.run(plan, policy, task_cancellation, on_update)).await
            }
            .await;
            match result {
                Ok(record) => {
                    let _ = sender.send(Action::DataOperation(DataOperationAction::JobUpdated {
                        request_id,
                        record: Box::new(record),
                    }));
                }
                Err(error) => {
                    let _ = sender.send(Action::DataOperation(DataOperationAction::JobFailed {
                        request_id,
                        job_id,
                        message: sanitize_terminal_text(&error.to_string()),
                    }));
                }
            }
        });
        self.data_operations
            .tasks
            .insert(request_id, (cancellation, task));
    }

    pub(super) fn cancel_data_operation(&mut self, request_id: Uuid) {
        if let Some((cancellation, _)) = self.data_operations.tasks.get(&request_id) {
            cancellation.cancel();
        }
        let prepared = self.data_operations.prepared.clone();
        self.background_tasks.push(tokio::spawn(async move {
            prepared
                .lock()
                .await
                .retain(|_, (owner, _)| *owner != request_id);
        }));
    }

    pub(super) fn load_data_tasks(&mut self, request_id: Uuid) {
        let store = self.data_operations.store.clone();
        let registry = self.registry.clone();
        let project_root = self.data_operations.project_root.clone();
        let sender = self.event_sender.clone();
        self.background_tasks.push(tokio::spawn(async move {
            let visible = {
                let registry = registry.lock().await;
                registry
                    .profiles
                    .values()
                    .filter(|profile| {
                        profile.access == crate::profile::ProfileAccess::Global
                            || project_root
                                .as_ref()
                                .is_some_and(|root| profile.access.contains_project(root))
                            || !registry.persisted.contains(&profile.id)
                    })
                    .map(|profile| profile.id)
                    .collect::<HashSet<_>>()
            };
            let result = tokio::task::spawn_blocking(move || store.list())
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()))
                .map(|records| {
                    records
                        .into_iter()
                        .filter(|record| {
                            record
                                .plan
                                .spec
                                .source
                                .iter()
                                .chain(record.plan.spec.destination.iter())
                                .all(|endpoint| visible.contains(&endpoint.profile_id))
                        })
                        .collect()
                })
                .map_err(|message| sanitize_terminal_text(&message));
            let _ = sender.send(Action::DataOperation(DataOperationAction::TasksLoaded {
                request_id,
                result,
            }));
        }));
    }

    pub(super) async fn shutdown_data_operations(&mut self) {
        for (cancellation, _) in self.data_operations.tasks.values() {
            cancellation.cancel();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        for (_, (_, mut task)) in self.data_operations.tasks.drain() {
            if tokio::time::timeout_at(deadline, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
        self.data_operations.prepared.lock().await.clear();
    }
}
