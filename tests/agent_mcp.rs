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

fn sqlite_profile(
    name: &str,
    read_only: bool,
    environment: lazydb::profile::Environment,
) -> lazydb::profile::ConnectionProfile {
    let mut profile = lazydb::profile::import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile;
    profile.read_only = read_only;
    profile.environment = environment;
    profile
}

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

#[test]
fn context_reports_server_policy_and_effective_write_capability() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();
    let service = AgentService::new(
        AgentProjectContext::resolve(Some(temp.path())).unwrap(),
        vec![sqlite_profile(
            "app-dev",
            false,
            lazydb::profile::Environment::Development,
        )],
        CredentialResolver::new(
            Arc::new(EmptySecrets),
            LocalCredentialStore::new(temp.path().join("key"), "test"),
        ),
    );
    let server = AgentMcpServer::new(service, WritePolicy::Deny);
    let context: serde_json::Value =
        serde_json::from_str(&server.context_json(None).unwrap()).unwrap();

    assert_eq!(context["server_write_policy"], "deny");
    assert_eq!(context["write_capability"]["allowed"], false);
    assert_eq!(
        context["write_capability"]["denial_reason"],
        "server_policy"
    );
    assert!(
        context["write_capability"]["message"]
            .as_str()
            .unwrap()
            .contains("--write-policy non-production")
    );
}

#[test]
fn context_reports_allowed_and_profile_or_environment_denials() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();
    let resolver = || {
        CredentialResolver::new(
            Arc::new(EmptySecrets),
            LocalCredentialStore::new(temp.path().join("key"), "test"),
        )
    };

    let allowed = AgentMcpServer::new(
        AgentService::new(
            AgentProjectContext::resolve(Some(temp.path())).unwrap(),
            vec![sqlite_profile(
                "app-dev",
                false,
                lazydb::profile::Environment::Development,
            )],
            resolver(),
        ),
        WritePolicy::NonProduction,
    );
    let context: serde_json::Value =
        serde_json::from_str(&allowed.context_json(None).unwrap()).unwrap();
    assert_eq!(context["server_write_policy"], "non-production");
    assert_eq!(context["write_capability"]["allowed"], true);
    assert_eq!(
        context["write_capability"]["denial_reason"],
        serde_json::Value::Null
    );
    assert_eq!(
        context["write_capability"]["message"],
        serde_json::Value::Null
    );

    let read_only = AgentMcpServer::new(
        AgentService::new(
            AgentProjectContext::resolve(Some(temp.path())).unwrap(),
            vec![sqlite_profile(
                "read-only",
                true,
                lazydb::profile::Environment::Development,
            )],
            resolver(),
        ),
        WritePolicy::All,
    );
    let context: serde_json::Value =
        serde_json::from_str(&read_only.context_json(None).unwrap()).unwrap();
    assert_eq!(
        context["write_capability"]["denial_reason"],
        "profile_read_only"
    );

    let production = AgentMcpServer::new(
        AgentService::new(
            AgentProjectContext::resolve(Some(temp.path())).unwrap(),
            vec![sqlite_profile(
                "production",
                false,
                lazydb::profile::Environment::Production,
            )],
            resolver(),
        ),
        WritePolicy::NonProduction,
    );
    let context: serde_json::Value =
        serde_json::from_str(&production.context_json(None).unwrap()).unwrap();
    assert_eq!(
        context["write_capability"]["denial_reason"],
        "production_policy"
    );
}
