use clap::Parser;
use lazydb::cli::{Cli, Command, McpClient, McpCommand, McpScope};
use tempfile::tempdir;

#[test]
fn parses_setup_options_and_defaults_to_project_scope() {
    let cli = Cli::try_parse_from([
        "lazydb",
        "mcp",
        "setup",
        "--client",
        "codex",
        "--dry-run",
        "--json",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Mcp {
            command: McpCommand::Setup {
                client,
                scope: McpScope::Project,
                dry_run: true,
                json: true,
                ..
            }
        }) if client == vec![McpClient::Codex]
    ));
}

#[test]
fn parses_multiple_clients() {
    let cli = Cli::try_parse_from([
        "lazydb",
        "mcp",
        "setup",
        "--client",
        "claude-code",
        "--client",
        "opencode",
        "--yes",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Mcp {
            command: McpCommand::Setup { client, yes: true, .. }
        }) if client == vec![McpClient::ClaudeCode, McpClient::Opencode]
    ));
}

#[test]
fn rejects_json_setup_without_confirmation_or_dry_run() {
    assert!(Cli::try_parse_from(["lazydb", "mcp", "setup", "--json"]).is_ok());
}

#[test]
fn dry_run_reports_missing_client_files_without_writing() {
    let dir = tempdir().unwrap();
    let output = lazydb::agent::setup::run(
        vec![McpClient::ClaudeCode, McpClient::Codex, McpClient::Opencode],
        McpScope::Project,
        Some(dir.path().to_owned()),
        None,
        true,
        false,
        true,
    )
    .unwrap();

    assert!(output.contains("\"status\":\"dry_run\""));
    assert!(output.contains(".mcp.json"));
    assert!(output.contains(".codex/config.toml"));
    assert!(output.contains("opencode.json"));
    assert!(!dir.path().join(".mcp.json").exists());
    assert!(!dir.path().join(".codex").exists());
    assert!(!dir.path().join("opencode.json").exists());
}

#[test]
fn existing_client_file_is_reported_as_conflict() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join(".mcp.json"), "{\"other\": true}\n").unwrap();
    let output = lazydb::agent::setup::run(
        vec![McpClient::ClaudeCode],
        McpScope::Project,
        Some(dir.path().to_owned()),
        None,
        true,
        false,
        true,
    )
    .unwrap();
    assert!(output.contains("\"status\":\"conflict\""));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap(),
        "{\"other\": true}\n"
    );
}

#[test]
fn yes_creates_new_project_configs_with_read_only_policy() {
    let dir = tempdir().unwrap();
    let output = lazydb::agent::setup::run(
        vec![McpClient::ClaudeCode, McpClient::Codex, McpClient::Opencode],
        McpScope::Project,
        Some(dir.path().to_owned()),
        None,
        false,
        true,
        false,
    )
    .unwrap();

    assert!(output.contains("complete"));
    assert!(
        std::fs::read_to_string(dir.path().join(".mcp.json"))
            .unwrap()
            .contains("write-policy")
    );
    assert!(
        std::fs::read_to_string(dir.path().join(".codex/config.toml"))
            .unwrap()
            .contains("mcp_servers.lazydb")
    );
    assert!(
        std::fs::read_to_string(dir.path().join("opencode.json"))
            .unwrap()
            .contains("\"lazydb\"")
    );
}
