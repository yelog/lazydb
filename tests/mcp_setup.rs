use clap::Parser;
use lazydb::cli::{Cli, Command, McpClient, McpCommand, McpScope};
use tempfile::tempdir;

#[test]
fn parses_setup_options_with_unspecified_scope() {
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
                scope: None,
                dry_run: true,
                json: true,
                ..
            }
        }) if client == vec![McpClient::Codex]
    ));
}

fn explicit_setup(client: McpClient, path: &std::path::Path, project: &std::path::Path) -> String {
    lazydb::agent::setup::run_with_options(lazydb::agent::setup::SetupOptions {
        clients: vec![client],
        scope: Some(McpScope::User),
        client_config: Some(path.into()),
        project: Some(project.into()),
        config: None,
        dry_run: false,
        yes: true,
        json: true,
    })
    .unwrap()
}

#[test]
fn preserves_jsonc_and_is_idempotent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("opencode.jsonc");
    let original = "{\n  // keep this comment\n  \"model\": \"custom\",\n  \"mcp\": {\n    \"other\": { \"type\": \"local\", \"command\": [\"other\"], },\n  },\n}\n";
    std::fs::write(&path, original).unwrap();
    assert!(
        explicit_setup(McpClient::Opencode, &path, dir.path()).contains("\"status\":\"added\"")
    );
    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.contains("// keep this comment"));
    assert!(updated.contains("\"other\": { \"type\": \"local\", \"command\": [\"other\"], }"));
    assert!(
        explicit_setup(McpClient::Opencode, &path, dir.path()).contains("\"status\":\"unchanged\"")
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), updated);
    assert!(!dir.path().join("opencode.json").exists());
}

#[test]
fn recognizes_and_preserves_native_opencode_v2_layout() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("opencode.jsonc");
    std::fs::write(
        &path,
        "{\n  \"mcp\": {\n    \"servers\": {\n      \"lazydb\": {\n        \"type\": \"local\",\n        \"command\": [\"lazydb\", \"mcp\", \"serve\", \"--project\", \".\", \"--write-policy\", \"deny\"],\n        \"cwd\": \".\"\n      }\n    }\n  }\n}\n",
    )
    .unwrap();

    let first = explicit_setup(McpClient::Opencode, &path, dir.path());
    assert!(first.contains("unchanged"), "{first}");
    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.contains("\"servers\""));
    assert!(!updated.contains("\"mcp\": {\n    \"lazydb\""));
}

#[test]
fn preserves_toml_comments_and_does_not_require_server() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let original = "# personal preferences\nmodel = 'custom' # retain quotes\n\n[mcp_servers.other]\ncommand = 'other'\n";
    std::fs::write(&path, original).unwrap();
    explicit_setup(McpClient::Codex, &path, dir.path());
    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.starts_with(original));
    let parsed: toml::Value = toml::from_str(&updated).unwrap();
    assert!(parsed["mcp_servers"]["lazydb"].get("required").is_none());
    assert!(explicit_setup(McpClient::Codex, &path, dir.path()).contains("unchanged"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), updated);
}

#[test]
fn explicit_project_scope_uses_project_directory_without_using_cwd() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("target-project");
    std::fs::create_dir_all(&project).unwrap();
    let config = project.join(".mcp.json");
    let result = lazydb::agent::setup::run_with_options(lazydb::agent::setup::SetupOptions {
        clients: vec![McpClient::ClaudeCode],
        scope: Some(McpScope::Project),
        client_config: Some(config.clone()),
        project: Some(project.clone()),
        config: None,
        dry_run: false,
        yes: true,
        json: true,
    });
    assert!(result.is_ok());
    assert!(config.exists());
}

#[test]
fn never_overwrites_conflicting_or_invalid_configuration() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("opencode.json");
    for (original, status) in [
        (
            "{\"mcp\":{\"lazydb\":{\"type\":\"local\",\"command\":[\"custom\"]}}}",
            "conflict",
        ),
        ("{\"mcp\":[]}", "invalid"),
        ("not json", "invalid"),
    ] {
        std::fs::write(&path, original).unwrap();
        let output = explicit_setup(McpClient::Opencode, &path, dir.path());
        assert!(
            output.contains(&format!("\"status\":\"{status}\"")),
            "{output}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}

#[test]
fn claude_local_only_updates_current_project_node() {
    let dir = tempdir().unwrap();
    let project = dir.path().canonicalize().unwrap();
    let path = dir.path().join(".claude.json");
    std::fs::write(
        &path,
        "{\"preferences\":true,\"projects\":{\"/another\":{\"allowedTools\":[\"Read\"]}}}",
    )
    .unwrap();
    let output = lazydb::agent::setup::run_with_options(lazydb::agent::setup::SetupOptions {
        clients: vec![McpClient::ClaudeCode],
        scope: Some(McpScope::Local),
        client_config: Some(path.clone()),
        project: Some(project.clone()),
        config: None,
        dry_run: false,
        yes: true,
        json: true,
    })
    .unwrap();
    assert!(output.contains("added"));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(value["preferences"], true);
    assert_eq!(value["projects"]["/another"]["allowedTools"][0], "Read");
    assert_eq!(
        value["projects"][project.to_str().unwrap()]["mcpServers"]["lazydb"]["command"],
        "lazydb"
    );
    assert!(value.get("mcpServers").is_none());
}

#[test]
fn rejects_unsupported_scope_and_multiple_explicit_clients() {
    let dir = tempdir().unwrap();
    assert!(
        lazydb::agent::setup::run(
            vec![McpClient::Codex],
            McpScope::Local,
            Some(dir.path().into()),
            None,
            true,
            false,
            true
        )
        .is_err()
    );
    assert!(
        lazydb::agent::setup::run_with_options(lazydb::agent::setup::SetupOptions {
            clients: vec![McpClient::Codex, McpClient::Opencode],
            scope: None,
            client_config: Some(dir.path().join("config")),
            project: Some(dir.path().into()),
            config: None,
            dry_run: true,
            yes: false,
            json: true,
        })
        .is_err()
    );
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
fn existing_client_file_is_planned_for_incremental_registration() {
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
    assert!(output.contains("\"status\":\"add\""));
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
