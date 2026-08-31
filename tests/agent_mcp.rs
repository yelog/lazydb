use async_trait::async_trait;
use lazydb::agent::mcp::AgentMcpServer;
use lazydb::agent::policy::WritePolicy;
use lazydb::agent::{context::AgentProjectContext, service::AgentService};
use lazydb::persistence::{
    credentials::CredentialResolver,
    local_credentials::LocalCredentialStore,
    secrets::{SecretStore, SecretStoreError},
};
use secrecy::SecretString;
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

struct EmptySecrets;
#[async_trait]
impl SecretStore for EmptySecrets {
    async fn available(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }
    async fn get(&self, _: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        Ok(None)
    }
    async fn set(&self, _: Uuid, _: &SecretString) -> Result<(), SecretStoreError> {
        Ok(())
    }
    async fn delete(&self, _: Uuid) -> Result<(), SecretStoreError> {
        Ok(())
    }
}

#[test]
fn tool_definitions_are_split_between_read_and_write_operations() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();
    let service = AgentService::new(
        AgentProjectContext::resolve(Some(temp.path())).unwrap(),
        Vec::new(),
        CredentialResolver::new(
            Arc::new(EmptySecrets),
            LocalCredentialStore::new(temp.path().join("key"), "test"),
        ),
    );
    let _server = AgentMcpServer::new(service, WritePolicy::Deny);
    assert_eq!(AgentMcpServer::get_context_tool_attr().name, "get_context");
    assert_eq!(
        AgentMcpServer::list_connections_tool_attr().name,
        "list_connections"
    );
    assert_eq!(
        AgentMcpServer::search_schema_tool_attr().name,
        "search_schema"
    );
    assert_eq!(
        AgentMcpServer::describe_object_tool_attr().name,
        "describe_object"
    );
    assert_eq!(AgentMcpServer::query_tool_attr().name, "query");
    assert_eq!(
        AgentMcpServer::execute_change_tool_attr().name,
        "execute_change"
    );
    assert_eq!(
        AgentMcpServer::execute_file_tool_attr().name,
        "execute_file"
    );
    assert_eq!(
        AgentMcpServer::query_tool_attr()
            .annotations
            .unwrap()
            .read_only_hint,
        Some(true)
    );
    assert_eq!(
        AgentMcpServer::execute_change_tool_attr()
            .annotations
            .unwrap()
            .destructive_hint,
        Some(true)
    );
}
