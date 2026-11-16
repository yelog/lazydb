use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars;
use rmcp::schemars::JsonSchema;
use rmcp::{
    ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    data_ops::{
        DataEndpoint, DataJobPlan, DataJobService, DataJobSpec, DataOperation, NativeToolPaths,
    },
    profile::DatabaseKind,
};

use super::{
    policy::{WriteCapability, WritePolicy, write_capability},
    service::AgentService,
};

#[derive(Clone)]
pub struct AgentMcpServer {
    service: Arc<AgentService>,
    policy: WritePolicy,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct QueryInput {
    pub connection: Option<String>,
    pub sql: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MongoQueryInput {
    pub connection: Option<String>,
    pub database: String,
    /// One supported read-only Mongo command encoded as a single JSON object.
    pub command: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MongoCollectionInput {
    pub connection: Option<String>,
    pub database: String,
    pub collection: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MongoChangeInput {
    pub connection: Option<String>,
    pub database: String,
    /// One supported Mongo insert, update, or delete command as ordered JSON text.
    pub command: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ContextInput {
    pub connection: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SchemaSearchInput {
    pub connection: Option<String>,
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DescribeInput {
    pub connection: Option<String>,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FileInput {
    pub connection: Option<String>,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataOperationInput {
    Backup,
    Restore,
    Copy,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataEndpointInput {
    pub profile_id: String,
    pub database: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanDataOperationInput {
    pub operation: DataOperationInput,
    pub source: Option<DataEndpointInput>,
    pub destination: Option<DataEndpointInput>,
    pub artifact: String,
}

impl TryFrom<PlanDataOperationInput> for DataJobSpec {
    type Error = String;

    fn try_from(input: PlanDataOperationInput) -> Result<Self, Self::Error> {
        let endpoint = |input: DataEndpointInput| {
            Ok::<_, String>(DataEndpoint {
                profile_id: Uuid::parse_str(&input.profile_id)
                    .map_err(|_| "The task references an unavailable connection.".to_owned())?,
                database: input.database,
            })
        };
        Ok(Self {
            operation: match input.operation {
                DataOperationInput::Backup => DataOperation::Backup,
                DataOperationInput::Restore => DataOperation::Restore,
                DataOperationInput::Copy => DataOperation::Copy,
            },
            source: input.source.map(endpoint).transpose()?,
            destination: input.destination.map(endpoint).transpose()?,
            artifact: PathBuf::from(input.artifact),
            tools: NativeToolPaths::default(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDataTasksInput {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetDataTaskInput {
    pub id: String,
}

#[derive(Serialize)]
struct McpDataOperationPlan {
    review_only: bool,
    execution_available: bool,
    message: &'static str,
    plan: DataJobPlan,
}

#[derive(Debug, Serialize)]
struct McpContext {
    #[serde(flatten)]
    context: crate::agent::types::AgentContext,
    server_write_policy: &'static str,
    write_capability: McpWriteCapability,
}

#[derive(Debug, Serialize)]
struct McpWriteCapability {
    allowed: bool,
    denial_reason: Option<&'static str>,
    message: Option<String>,
}

impl AgentMcpServer {
    pub fn new(service: AgentService, policy: WritePolicy) -> Self {
        Self {
            service: Arc::new(service),
            policy,
            tool_router: Self::tool_router(),
        }
    }

    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let agent_service = self.service.clone();
        let service = match ServiceExt::serve(self, stdio()).await {
            Ok(service) => service,
            Err(error) => {
                agent_service.shutdown().await;
                return Err(error.into());
            }
        };
        let result = service.waiting().await;
        agent_service.shutdown().await;
        result?;
        Ok(())
    }

    pub fn context_json(&self, connection: Option<&str>) -> Result<String, String> {
        let selected = self
            .service
            .select(connection)
            .map_err(|error| error.message)?;
        let context = self
            .service
            .context(connection)
            .map_err(|error| error.message)?;
        let write_capability = match write_capability(selected.profile, self.policy) {
            WriteCapability::Allowed => McpWriteCapability {
                allowed: true,
                denial_reason: None,
                message: None,
            },
            WriteCapability::Denied(error) => McpWriteCapability {
                allowed: false,
                denial_reason: Some(error.reason()),
                message: Some(error.to_string()),
            },
        };
        serde_json::to_string(&McpContext {
            context,
            server_write_policy: self.policy.as_str(),
            write_capability,
        })
        .map_err(|error| error.to_string())
    }

    fn scope_data_paths(
        &self,
        service: &DataJobService,
        spec: &mut DataJobSpec,
        require_inputs: bool,
    ) -> Result<(), String> {
        if !service.can_view(spec) {
            return Err("The task references an unavailable connection.".to_owned());
        }
        spec.artifact = project_data_path(
            self.service.project_root(),
            &spec.artifact,
            require_inputs && spec.operation == DataOperation::Restore,
        )?;
        for (endpoint, source) in [
            (spec.source.as_mut(), true),
            (spec.destination.as_mut(), false),
        ] {
            let Some(endpoint) = endpoint else {
                continue;
            };
            let profile = service
                .profiles()
                .iter()
                .find(|profile| profile.id == endpoint.profile_id)
                .ok_or_else(|| "The task references an unavailable connection.".to_owned())?;
            if profile.kind == DatabaseKind::Sqlite {
                endpoint.database = project_data_path(
                    self.service.project_root(),
                    Path::new(&endpoint.database),
                    require_inputs && source,
                )?
                .to_str()
                .ok_or_else(|| "SQLite data-operation paths must be valid UTF-8.".to_owned())?
                .to_owned();
            }
        }
        Ok(())
    }

    fn data_result_json(&self, result: &impl Serialize) -> Result<String, String> {
        let json = serde_json::to_string(result).map_err(|error| error.to_string())?;
        if json.len() > self.service.max_result_bytes {
            return Err("The data-task result exceeds the server result-size limit.".to_owned());
        }
        Ok(json)
    }
}

#[tool_router(router = tool_router)]
impl AgentMcpServer {
    /// Return the current project and selected LazyDB connection context.
    #[tool(name = "get_context", annotations(read_only_hint = true))]
    async fn get_context(&self, input: Parameters<ContextInput>) -> Result<String, String> {
        let connection = input.0.connection.filter(|value| !value.is_empty());
        self.context_json(connection.as_deref())
    }

    /// List current-project and global connections visible to this server.
    #[tool(name = "list_connections", annotations(read_only_hint = true))]
    async fn list_connections(&self) -> Result<String, String> {
        serde_json::to_string(&self.service.connections()).map_err(|e| e.to_string())
    }

    #[tool(
        name = "plan_data_operation",
        description = "Review a native backup, restore, or same-engine copy plan. This tool only performs read-only preflight; it does not create archives or databases or execute the plan. Artifact and SQLite paths must be inside the current project; relative paths use the project root. Destination restrictions from --write-policy and the profile apply. Native executables cannot be selected through MCP. Execute a reviewed operation separately through the LazyDB CLI or TUI.",
        annotations(read_only_hint = true, destructive_hint = false)
    )]
    async fn plan_data_operation(
        &self,
        input: Parameters<PlanDataOperationInput>,
    ) -> Result<String, String> {
        let mut spec = DataJobSpec::try_from(input.0)?;
        let service = self
            .service
            .data_job_service()
            .map_err(|error| error.to_string())?;
        if !service.can_view(&spec) {
            return Err("The task references an unavailable connection.".to_owned());
        }
        if let Some(destination) = &spec.destination {
            let profile = service
                .profiles()
                .iter()
                .find(|profile| profile.id == destination.profile_id)
                .ok_or_else(|| "The task references an unavailable connection.".to_owned())?;
            if let WriteCapability::Denied(error) = write_capability(profile, self.policy) {
                return Err(error.to_string());
            }
        }
        self.scope_data_paths(&service, &mut spec, true)?;
        let plan = service
            .plan(spec, self.policy, CancellationToken::new())
            .await
            .map_err(|error| error.to_string())?;
        self.data_result_json(&McpDataOperationPlan {
            review_only: true,
            execution_available: false,
            message: "Review only: no backup, restore, or copy was executed; no output artifact or destination database was created. Use the LazyDB CLI or TUI to execute a reviewed operation.",
            plan,
        })
    }

    #[tool(
        name = "list_data_tasks",
        description = "List recent native data-task records whose connections and artifact/SQLite paths are visible in the current project. Returns up to 50 tasks by default, at most 100. Does not execute any data operation.",
        annotations(read_only_hint = true, destructive_hint = false)
    )]
    async fn list_data_tasks(
        &self,
        input: Parameters<ListDataTasksInput>,
    ) -> Result<String, String> {
        let service = self
            .service
            .data_job_service()
            .map_err(|_| "Data-task history is unavailable.".to_owned())?;
        let tasks = service
            .store()
            .list()
            .map_err(|_| "Data-task history is unavailable.".to_owned())?;
        let limit = input.0.limit.unwrap_or(50).clamp(1, 100);
        let tasks = tasks
            .into_iter()
            .filter(|task| {
                self.scope_data_paths(&service, &mut task.plan.spec.clone(), false)
                    .is_ok()
            })
            .take(limit)
            .collect::<Vec<_>>();
        self.data_result_json(&tasks)
    }

    #[tool(
        name = "get_data_task",
        description = "Inspect a native data task visible in the current project, including its status and verification. Hidden and missing tasks are unavailable. Does not execute or rerun the task.",
        annotations(read_only_hint = true, destructive_hint = false)
    )]
    async fn get_data_task(&self, input: Parameters<GetDataTaskInput>) -> Result<String, String> {
        let unavailable = || "The data task is unavailable.".to_owned();
        let id = Uuid::parse_str(&input.0.id).map_err(|_| unavailable())?;
        let service = self.service.data_job_service().map_err(|_| unavailable())?;
        let task = service.store().load(id).map_err(|_| unavailable())?;
        self.scope_data_paths(&service, &mut task.plan.spec.clone(), false)
            .map_err(|_| unavailable())?;
        self.data_result_json(&task)
    }

    /// Search the selected connection's schema catalog.
    #[tool(name = "search_schema", annotations(read_only_hint = true))]
    async fn search_schema(&self, input: Parameters<SchemaSearchInput>) -> Result<String, String> {
        let input = input.0;
        let result = self
            .service
            .search_schema(
                input.connection.as_deref(),
                input.query,
                input.limit.unwrap_or(100),
            )
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    /// Describe one catalog object using the progressive schema interface.
    #[tool(name = "describe_object", annotations(read_only_hint = true))]
    async fn describe_object(&self, input: Parameters<DescribeInput>) -> Result<String, String> {
        let input = input.0;
        let result = self
            .service
            .search_schema(input.connection.as_deref(), input.name, 1)
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&result).map_err(|error| error.to_string())
    }

    /// Execute a project-local SQL file subject to LazyDB write policy. Client
    /// approval does not override --write-policy, a read-only profile,
    /// production restrictions, or database grants.
    #[tool(
        name = "execute_file",
        description = "Execute a project-local SQL file subject to LazyDB write policy. Client approval does not override --write-policy, a read-only profile, production restrictions, or database grants.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn execute_file(&self, input: Parameters<FileInput>) -> Result<String, String> {
        let input = input.0;
        let path = std::path::PathBuf::from(input.path)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !path.starts_with(self.service.project_root()) {
            return Err("SQL file is outside the project root".to_owned());
        }
        let sql = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let result = self
            .service
            .execute(input.connection.as_deref(), &sql, self.policy)
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&result).map_err(|error| error.to_string())
    }

    /// Execute a read-only query against the selected connection.
    #[tool(name = "query", annotations(read_only_hint = true))]
    async fn query(&self, input: Parameters<QueryInput>) -> Result<String, String> {
        let input = input.0;
        let result = self
            .service
            .query(input.connection.as_deref(), &input.sql)
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    /// Execute a supported read-only Mongo command and return Canonical EJSON.
    #[tool(name = "mongo_query", annotations(read_only_hint = true))]
    async fn mongo_query(&self, input: Parameters<MongoQueryInput>) -> Result<String, String> {
        let input = input.0;
        let result = self
            .service
            .mongo_query(input.connection.as_deref(), &input.database, &input.command)
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&result).map_err(|error| error.to_string())
    }

    /// Execute an explicitly authorized MongoDB change. Server write policy and profile restrictions always apply.
    #[tool(
        name = "mongo_execute_change",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn mongo_execute_change(
        &self,
        input: Parameters<MongoChangeInput>,
    ) -> Result<String, String> {
        let input = input.0;
        let result = self
            .service
            .mongo_execute_change(
                input.connection.as_deref(),
                &input.database,
                &input.command,
                self.policy,
            )
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&result).map_err(|error| error.to_string())
    }

    #[tool(name = "mongo_list_collections", annotations(read_only_hint = true))]
    async fn mongo_list_collections(
        &self,
        input: Parameters<MongoCollectionInput>,
    ) -> Result<String, String> {
        let input = input.0;
        let collections = self
            .service
            .mongo_list_collections(input.connection.as_deref(), &input.database)
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&collections).map_err(|error| error.to_string())
    }

    #[tool(name = "mongo_describe_collection", annotations(read_only_hint = true))]
    async fn mongo_describe_collection(
        &self,
        input: Parameters<MongoCollectionInput>,
    ) -> Result<String, String> {
        let input = input.0;
        let collection = input
            .collection
            .ok_or_else(|| "collection is required".to_owned())?;
        let description = self
            .service
            .mongo_describe_collection(input.connection.as_deref(), &input.database, &collection)
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&description).map_err(|error| error.to_string())
    }

    /// Execute a SQL change subject to LazyDB write policy. Client approval
    /// does not override --write-policy, a read-only profile, production
    /// restrictions, or database grants. Call get_context first to inspect
    /// effective capability.
    #[tool(
        name = "execute_change",
        description = "Execute a SQL change subject to LazyDB write policy. Client approval does not override --write-policy, a read-only profile, production restrictions, or database grants. Call get_context first to inspect effective capability.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn execute_change(&self, input: Parameters<QueryInput>) -> Result<String, String> {
        let input = input.0;
        let result = self
            .service
            .execute(input.connection.as_deref(), &input.sql, self.policy)
            .await
            .map_err(|error| error.message)?;
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }
}

fn project_data_path(root: &Path, path: &Path, require_existing: bool) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.to_string_lossy().chars().any(char::is_control) {
        return Err(
            "A data-operation file path without control characters is required.".to_owned(),
        );
    }
    let absolute = root.join(path);
    let resolved = match std::fs::symlink_metadata(&absolute) {
        Ok(_) => absolute
            .canonicalize()
            .map_err(|_| "Cannot resolve the data-operation file path.".to_owned())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !require_existing => {
            let name = absolute
                .file_name()
                .ok_or_else(|| "A data-operation file name is required.".to_owned())?;
            absolute
                .parent()
                .ok_or_else(|| "A data-operation parent directory is required.".to_owned())?
                .canonicalize()
                .map_err(|_| "The data-operation parent directory must already exist.".to_owned())?
                .join(name)
        }
        Err(_) => return Err("The data-operation input file is unavailable.".to_owned()),
    };
    if !resolved.starts_with(root) {
        return Err("The data-operation path is outside the project root.".to_owned());
    }
    Ok(resolved)
}

#[tool_handler(router = self.tool_router)]
impl rmcp::ServerHandler for AgentMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("lazydb", env!("CARGO_PKG_VERSION")))
    }
}

pub async fn run(
    project: Option<PathBuf>,
    connection: Option<String>,
    policy: WritePolicy,
    config: Option<PathBuf>,
) -> anyhow::Result<()> {
    let project = server_project(
        project,
        std::env::var_os("CLAUDE_PROJECT_DIR").map(PathBuf::from),
    );
    let service = AgentService::load(project.as_deref(), config)?;
    if let Some(selector) = connection.as_deref() {
        service.select(Some(selector))?;
    }
    AgentMcpServer::new(service, policy).serve_stdio().await
}

fn server_project(explicit: Option<PathBuf>, client_project: Option<PathBuf>) -> Option<PathBuf> {
    explicit.or_else(|| client_project.filter(|p| p.is_absolute()))
}

#[cfg(test)]
mod project_tests {
    use super::*;

    #[test]
    fn resolves_each_client_project_at_startup_without_pinning_setup_directory() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["project-a", "project-b"] {
            let project = dir.path().join(name);
            std::fs::create_dir_all(&project).unwrap();
            let selected = server_project(None, Some(project.clone()));
            let context =
                crate::agent::context::AgentProjectContext::resolve(selected.as_deref()).unwrap();
            assert_eq!(context.root(), project.canonicalize().unwrap());
        }
        assert_eq!(
            server_project(Some("explicit".into()), Some(dir.path().into())),
            Some("explicit".into())
        );
        assert_eq!(server_project(None, Some("relative".into())), None);
    }
}
