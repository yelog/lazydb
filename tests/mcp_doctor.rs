use lazydb::agent::doctor;
use lazydb::cli::McpClient;
use tempfile::tempdir;

#[tokio::test]
async fn reports_missing_configs_without_database_io() {
    let dir = tempdir().unwrap();
    let output = doctor::run(
        vec![McpClient::ClaudeCode],
        Some(dir.path().to_owned()),
        None,
        false,
        true,
    )
    .await
    .unwrap();
    assert!(output.contains("\"status\":\"warning\""));
    assert!(output.contains("missing"));
    assert!(output.contains("database I/O was not performed"));
}

#[tokio::test]
async fn recognizes_generated_read_only_configuration() {
    let dir = tempdir().unwrap();
    lazydb::agent::setup::run(
        vec![McpClient::Codex],
        lazydb::cli::McpScope::Project,
        Some(dir.path().to_owned()),
        None,
        false,
        true,
        false,
    )
    .unwrap();
    let output = doctor::run(
        vec![McpClient::Codex],
        Some(dir.path().to_owned()),
        None,
        false,
        true,
    )
    .await
    .unwrap();
    assert!(output.contains("\"status\":\"ok\""));
    assert!(output.contains("deny policy"));
}

#[tokio::test]
async fn probe_is_explicit_and_does_not_start_client_commands_yet() {
    let dir = tempdir().unwrap();
    let output = doctor::run(
        vec![McpClient::Opencode],
        Some(dir.path().to_owned()),
        None,
        true,
        true,
    )
    .await
    .unwrap();
    assert!(output.contains("not implemented"));
    assert!(output.contains("no configured client was started"));
}
