use std::{path::PathBuf, sync::Arc};

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
        let service = ServiceExt::serve(self, stdio()).await?;
        service.waiting().await?;
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
