use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, OnceCell};

use crate::sql::CompletionIndex;
use crate::{
    db::{
        DatabaseConnection,
        catalog::{
            CatalogEntry, CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogTarget,
            ObjectGroup,
        },
    },
    identity::ConnectionIdentity,
    persistence::{
        credentials::CredentialResolver, local_credentials::LocalCredentialStore, paths::AppPaths,
        profiles::ProfileStore, secrets::NativeSecretStore,
    },
    profile::{CatalogScope, ConnectionProfile},
};

#[async_trait::async_trait]
pub trait CatalogTargetLoader: Send + Sync {
    async fn load(
        &self,
        target: CatalogTarget,
        scope: &CatalogScope,
    ) -> anyhow::Result<Vec<CatalogEntry>>;
}

#[derive(Clone, Debug, Default)]
pub struct CatalogLoadStatus {
    pub complete: bool,
    pub errors: Vec<String>,
}

pub async fn discover_index(
    loader: &dyn CatalogTargetLoader,
    scope: &CatalogScope,
) -> (CompletionIndex, CatalogLoadStatus) {
    let mut entries = Vec::new();
    let mut status = CatalogLoadStatus {
        complete: true,
        errors: Vec::new(),
    };
    let databases = load_target_page(loader, scope, CatalogTarget::Databases, &mut status).await;
    entries.extend(databases.iter().cloned());
    for database in databases
        .iter()
        .filter(|entry| entry.kind == CatalogKind::Database)
    {
        let schemas = load_target_page(
            loader,
            scope,
            CatalogTarget::Schemas {
                database: database.id.clone(),
            },
            &mut status,
        )
        .await;
        entries.extend(schemas.iter().cloned());
        for schema in schemas
            .iter()
            .filter(|entry| entry.kind == CatalogKind::Schema)
        {
            for group in [
                ObjectGroup::Tables,
                ObjectGroup::Views,
                ObjectGroup::MaterializedViews,
            ] {
                let Ok(target) = CatalogTarget::objects(schema.id.clone(), group) else {
                    continue;
                };
                let relations = load_target_page(loader, scope, target, &mut status).await;
                for relation in relations.iter().filter(|entry| entry.kind.is_relation()) {
                    let Ok(target) = CatalogTarget::relation_children(relation.id.clone()) else {
                        continue;
                    };
                    let children = load_target_page(loader, scope, target, &mut status).await;
                    entries.push(relation.clone());
                    entries.extend(children);
                }
            }
        }
    }
    let mut index = CompletionIndex::default();
    index.replace_scoped(&entries, scope);
    (index, status)
}

async fn load_target_page(
    loader: &dyn CatalogTargetLoader,
    scope: &CatalogScope,
    target: CatalogTarget,
    status: &mut CatalogLoadStatus,
) -> Vec<CatalogEntry> {
    match loader.load(target.clone(), scope).await {
        Ok(entries) => entries,
        Err(error) => {
            if !is_unsupported_catalog_error(&error) {
                status
                    .errors
                    .push(format!("{} failed: {}", target.description(), error));
                status.complete = false;
            }
            Vec::new()
        }
    }
}

fn is_unsupported_catalog_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::db::DatabaseError>()
        .and_then(|error| error.code.as_deref())
        .is_some_and(|code| code == "catalog_target_unsupported")
}

pub const CATALOG_TTL: std::time::Duration = std::time::Duration::from_secs(60);
pub const CATALOG_RETRY_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(2);
pub const CATALOG_IO_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);
pub const CATALOG_MAX_CONCURRENCY: usize = 4;

type Clock = std::sync::Arc<dyn Fn() -> std::time::Instant + Send + Sync>;

#[derive(Clone, Debug)]
struct TargetLoad {
    entries: Arc<Vec<CatalogEntry>>,
    loaded_at: Option<std::time::Instant>,
    error: Option<String>,
    attempted_at: std::time::Instant,
}

#[derive(Clone, Debug)]
pub struct TargetLoadOutcome {
    pub entries: Arc<Vec<CatalogEntry>>,
    pub incomplete: bool,
}

pub struct CatalogTargetService {
    loader: Arc<dyn CatalogTargetLoader>,
    scope: CatalogScope,
    database: Option<String>,
    schema: Option<String>,
    targets: Mutex<HashMap<CatalogTarget, Arc<OnceCell<TargetLoad>>>>,
    clock: Clock,
}

impl std::fmt::Debug for CatalogTargetService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CatalogTargetService")
            .field("database", &self.database)
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl CatalogTargetService {
    pub fn new(
        loader: Arc<dyn CatalogTargetLoader>,
        scope: CatalogScope,
        database: Option<String>,
        schema: Option<String>,
    ) -> Arc<Self> {
        Arc::new(Self::with_clock(
            loader,
            scope,
            database,
            schema,
            std::sync::Arc::new(std::time::Instant::now),
        ))
    }

    pub fn with_clock(
        loader: Arc<dyn CatalogTargetLoader>,
        scope: CatalogScope,
        database: Option<String>,
        schema: Option<String>,
        clock: Clock,
    ) -> Self {
        Self {
            loader,
            scope,
            database,
            schema,
            targets: Mutex::new(HashMap::new()),
            clock,
        }
    }

    pub fn completion_context(&self) -> crate::sql::CompletionContext<'_> {
        crate::sql::CompletionContext {
            database: self.database.as_deref(),
            schema: self.schema.as_deref(),
        }
    }

    pub async fn ensure(self: &Arc<Self>, target: CatalogTarget) -> TargetLoadOutcome {
        loop {
            let cell = {
                let mut targets = self.targets.lock().await;
                let cell = targets
                    .entry(target.clone())
                    .or_insert_with(|| Arc::new(OnceCell::new()))
                    .clone();
                if let Some(state) = cell.get() {
                    let now = (self.clock)();
                    let retry_out = state.error.is_some()
                        && now.duration_since(state.attempted_at) >= CATALOG_RETRY_COOLDOWN;
                    let expired = state.error.is_none()
                        && state
                            .loaded_at
                            .is_none_or(|loaded_at| now.duration_since(loaded_at) >= CATALOG_TTL);
                    if retry_out || expired {
                        targets.remove(&target);
                        continue;
                    }
                    return TargetLoadOutcome {
                        entries: state.entries.clone(),
                        incomplete: state.error.is_some(),
                    };
                }
                cell
            };
            let state = self.load_cell(&cell, &target).await;
            let state = state.get().expect("cell loaded");
            return TargetLoadOutcome {
                entries: state.entries.clone(),
                incomplete: state.error.is_some(),
            };
        }
    }

    pub async fn ensure_many(
        self: &Arc<Self>,
        targets: Vec<CatalogTarget>,
        budget: std::time::Duration,
    ) -> (Vec<Arc<Vec<CatalogEntry>>>, bool) {
        let deadline = (self.clock)() + budget;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(CATALOG_MAX_CONCURRENCY));
        let mut results: Vec<Arc<Vec<CatalogEntry>>> = Vec::new();
        let mut incomplete = false;
        let mut awaited = targets
            .into_iter()
            .filter(|_| {
                let within = (self.clock)() < deadline;
                if !within {
                    incomplete = true;
                }
                within
            })
            .collect::<Vec<_>>();
        while !awaited.is_empty() {
            let batch = std::mem::take(&mut awaited);
            let mut tasks = tokio::task::JoinSet::new();
            for target in batch {
                let service = self.clone();
                let semaphore = semaphore.clone();
                tasks.spawn(async move {
                    let _permit = semaphore.acquire_owned().await.ok();
                    service.ensure(target).await
                });
            }
            let remaining = deadline.saturating_duration_since((self.clock)());
            if remaining.is_zero() {
                tasks.abort_all();
                incomplete = true;
                break;
            }
            match tokio::time::timeout(
                remaining,
                drain_targets(&mut tasks, &mut results, &mut incomplete),
            )
            .await
            {
                Ok(()) => {}
                Err(_) => {
                    tasks.abort_all();
                    incomplete = true;
                }
            }
        }
        (results, incomplete)
    }

    pub async fn warm(
        self: &Arc<Self>,
        context: crate::sql::CompletionContext<'_>,
        qualifiers: &[String],
    ) -> (Vec<CatalogEntry>, bool) {
        let mut targets = vec![CatalogTarget::Databases];
        let databases = self.ensure(CatalogTarget::Databases).await;
        let databases_incomplete = databases.incomplete;
        let database_names: Vec<CatalogId> = databases
            .entries
            .iter()
            .filter(|entry| entry.kind == CatalogKind::Database)
            .map(|entry| entry.id.clone())
            .collect();
        let mut current: Option<String> = context.database.map(str::to_owned);
        if current.is_none() {
            current = databases
                .entries
                .iter()
                .find(|entry| entry.kind == CatalogKind::Database)
                .map(|entry| entry.qualified_name.object.clone());
        }
        if let Some(current_name) = current
            && let Some(database) = database_names
                .iter()
                .find(|id| {
                    id.native_path
                        .first()
                        .is_some_and(|part| part.eq_ignore_ascii_case(&current_name))
                })
                .cloned()
        {
            targets.push(CatalogTarget::Schemas {
                database: database.clone(),
            });
            let schemas = self.ensure(CatalogTarget::Schemas { database }).await;
            for schema in schemas
                .entries
                .iter()
                .filter(|entry| entry.kind == CatalogKind::Schema)
            {
                for group in [
                    ObjectGroup::Tables,
                    ObjectGroup::Views,
                    ObjectGroup::MaterializedViews,
                ] {
                    if let Ok(target) = CatalogTarget::objects(schema.id.clone(), group) {
                        targets.push(target);
                    }
                }
            }
        }
        if !qualifiers.is_empty()
            && let Some(database) = database_names
                .iter()
                .find(|id| {
                    id.native_path
                        .first()
                        .is_some_and(|part| part.eq_ignore_ascii_case(&qualifiers[0]))
                })
                .cloned()
        {
            targets.push(CatalogTarget::Schemas {
                database: database.clone(),
            });
            if qualifiers.len() >= 2 {
                let schemas = self.ensure(CatalogTarget::Schemas { database }).await;
                for schema in schemas
                    .entries
                    .iter()
                    .filter(|entry| entry.kind == CatalogKind::Schema)
                    .filter(|entry| {
                        entry
                            .qualified_name
                            .object
                            .eq_ignore_ascii_case(&qualifiers[1])
                    })
                {
                    for group in [
                        ObjectGroup::Tables,
                        ObjectGroup::Views,
                        ObjectGroup::MaterializedViews,
                    ] {
                        if let Ok(target) = CatalogTarget::objects(schema.id.clone(), group) {
                            targets.push(target);
                        }
                    }
                }
            }
        }
        let (entries, incomplete) = self.ensure_many(targets, CATALOG_IO_BUDGET).await;
        let mut merged = databases.entries.as_ref().clone();
        for batch in entries {
            merged.extend(batch.as_ref().iter().cloned());
        }
        (merged, incomplete || databases_incomplete)
    }

    pub async fn load_children(
        self: &Arc<Self>,
        relations: &[CatalogId],
    ) -> (Vec<CatalogEntry>, bool) {
        let targets = relations
            .iter()
            .filter_map(|relation| CatalogTarget::relation_children(relation.clone()).ok())
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return (Vec::new(), false);
        }
        let (entries, incomplete) = self.ensure_many(targets, CATALOG_IO_BUDGET).await;
        let mut merged = Vec::new();
        for batch in entries {
            merged.extend(batch.as_ref().iter().cloned());
        }
        (merged, incomplete)
    }

    async fn load_cell(
        self: &Arc<Self>,
        cell: &Arc<OnceCell<TargetLoad>>,
        target: &CatalogTarget,
    ) -> Arc<OnceCell<TargetLoad>> {
        load_cell_inner(self, cell, target).await
    }
}

async fn drain_targets(
    tasks: &mut tokio::task::JoinSet<TargetLoadOutcome>,
    results: &mut Vec<Arc<Vec<CatalogEntry>>>,
    incomplete: &mut bool,
) {
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(outcome) => {
                results.push(outcome.entries.clone());
                if outcome.incomplete {
                    *incomplete = true;
                }
            }
            Err(_) => *incomplete = true,
        }
    }
}

async fn load_cell_inner(
    service: &Arc<CatalogTargetService>,
    cell: &Arc<OnceCell<TargetLoad>>,
    target: &CatalogTarget,
) -> Arc<OnceCell<TargetLoad>> {
    cell.get_or_init(|| async {
        let attempted_at = (service.clock)();
        match service.loader.load(target.clone(), &service.scope).await {
            Ok(entries) => TargetLoad {
                entries: Arc::new(entries),
                loaded_at: Some((service.clock)()),
                error: None,
                attempted_at,
            },
            Err(error) if is_unsupported_catalog_error(&error) => TargetLoad {
                entries: Arc::new(Vec::new()),
                loaded_at: None,
                error: None,
                attempted_at,
            },
            Err(error) => TargetLoad {
                entries: Arc::new(Vec::new()),
                loaded_at: None,
                error: Some(format!("{error:#}")),
                attempted_at,
            },
        }
    })
    .await;
    cell.clone()
}

#[derive(Clone, Debug)]
pub struct CatalogProvider {
    connection: Arc<DatabaseConnection>,
    profile: ConnectionProfile,
}

impl CatalogProvider {
    pub fn profile_id(&self) -> uuid::Uuid {
        self.profile.id
    }

    pub fn completion_context(&self) -> crate::sql::CompletionContext<'_> {
        crate::sql::CompletionContext {
            database: self.profile.database.as_deref(),
            schema: self.profile.default_schema.as_deref(),
        }
    }

    pub fn dialect(&self) -> crate::cli::LspDialect {
        crate::cli::LspDialect::from(self.profile.kind)
    }

    pub fn catalog_scope(&self) -> &CatalogScope {
        &self.profile.catalog_scope
    }

    pub async fn from_config(
        project: Option<&std::path::Path>,
        config: Option<std::path::PathBuf>,
        selector: Option<&str>,
    ) -> anyhow::Result<Option<Self>> {
        let project = crate::agent::context::AgentProjectContext::resolve(project)?;
        let paths = AppPaths::discover()?;
        let profile_store = ProfileStore::new(config.unwrap_or_else(|| paths.profiles_file()))
            .with_credential_key_path(paths.credential_key_file());
        let profiles = profile_store.load()?;
        let visible = project.visible_profiles(&profiles.profiles);
        let selected = crate::agent::selection::select_profile(&visible, selector)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let resolver = CredentialResolver::new(
            std::sync::Arc::new(NativeSecretStore),
            LocalCredentialStore::new(profile_store.credential_key_path(), "lazydb")
                .with_fallback_key_path(
                    profile_store
                        .path()
                        .parent()
                        .map(|parent| parent.join("credential.key"))
                        .unwrap_or_else(|| std::path::PathBuf::from("credential.key")),
                ),
        );
        let password = resolver.resolve_headless(selected.profile).await?;
        let connection = DatabaseConnection::connect(selected.profile, password.as_ref()).await?;
        Ok(Some(Self {
            connection: std::sync::Arc::new(connection),
            profile: selected.profile.clone(),
        }))
    }
}

#[async_trait::async_trait]
impl CatalogTargetLoader for CatalogProvider {
    async fn load(
        &self,
        target: CatalogTarget,
        scope: &CatalogScope,
    ) -> anyhow::Result<Vec<CatalogEntry>> {
        self.load_target(target, self.connection_identity(), scope.clone())
            .await
    }
}

impl CatalogProvider {
    fn connection_identity(&self) -> ConnectionIdentity {
        ConnectionIdentity {
            profile_id: self.profile.id,
            generation: 0,
        }
    }

    async fn load_target(
        &self,
        target: CatalogTarget,
        connection: ConnectionIdentity,
        scope: CatalogScope,
    ) -> anyhow::Result<Vec<CatalogEntry>> {
        let mut cursor = None;
        let mut request_id = 1;
        let mut entries = Vec::new();
        loop {
            let request = CatalogRequest {
                key: CatalogRequestKey {
                    connection,
                    catalog_epoch: 0,
                    request_id,
                    target: target.clone(),
                    cursor: cursor.clone(),
                },
                scope: scope.clone(),
                page_size: 500,
            };
            let page = self.connection.load_catalog_page(&request).await?;
            entries.extend(page.entries);
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
            request_id += 1;
        }
        Ok(entries)
    }
}
