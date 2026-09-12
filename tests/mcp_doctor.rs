use lazydb::agent::doctor;
use lazydb::cli::McpClient;
use tempfile::tempdir;

#[tokio::test]
async fn comments_are_not_servers_and_disabled_entries_are_reported() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("opencode.jsonc");
    for (text, detail) in [
        ("{ // lazydb --write-policy deny\n}", "missing LazyDB"),
        (
            "{\"mcp\":{\"lazydb\":{\"type\":\"local\",\"command\":[\"lazydb\",\"mcp\",\"serve\",\"--write-policy\",\"deny\"],\"enabled\":false}}}",
            "disabled",
        ),
        (
            "{\"mcp\":{\"lazydb\":{\"type\":\"remote\",\"url\":\"https://example.com/lazydb/write-policy/deny\"}}}",
            "could not be confirmed",
        ),
    ] {
        std::fs::write(&path, text).unwrap();
        let output = doctor::run_with_options(
            vec![McpClient::Opencode],
            Some(dir.path().into()),
            Some(path.clone()),
            false,
            true,
            None,
        )
        .await
        .unwrap();
        assert!(output.contains(detail), "{output}");
        let report: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(report["status"], "warning");
    }
}

#[tokio::test]
async fn reports_native_opencode_v2_disabled_entries() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("opencode.jsonc");
    std::fs::write(
        &path,
        r#"{"mcp":{"servers":{"lazydb":{"type":"local","command":["lazydb","mcp","serve","--write-policy","deny"],"disabled":true}}}}"#,
    )
    .unwrap();
    let output = doctor::run_with_options(
        vec![McpClient::Opencode],
        Some(dir.path().into()),
        Some(path),
        false,
        true,
        None,
    )
    .await
    .unwrap();
    assert!(output.contains("disabled"), "{output}");
    let report: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(report["status"], "warning");
}

#[tokio::test]
async fn reports_v2_shadowing_and_does_not_merge_server_objects() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("opencode.jsonc");
    std::fs::write(
        &path,
        r#"{"mcp":{"lazydb":{"type":"local","command":["lazydb","mcp","serve","--write-policy","deny"]},"servers":{"lazydb":{"type":"local","disabled":true}}}}"#,
    )
    .unwrap();
    let output = doctor::run_with_options(
        vec![McpClient::Opencode],
        Some(dir.path().into()),
        Some(path),
        false,
        true,
        None,
    )
    .await
    .unwrap();
    assert!(output.contains("disabled"), "{output}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["status"],
        "warning"
    );
}

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
async fn reports_explicit_opencode_runtime_when_requested() {
    let dir = tempdir().unwrap();
    let output = doctor::run_with_options(
        vec![McpClient::Opencode],
        Some(dir.path().to_owned()),
        None,
        false,
        true,
        Some("definitely-not-an-opencode-binary".into()),
    )
    .await
    .unwrap();
    assert!(output.contains("OpenCode runtime"), "{output}");
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
async fn probe_reports_missing_server_without_starting_a_command() {
    let dir = tempdir().unwrap();
    let config = dir.path().join("opencode.json");
    std::fs::write(&config, "{}\n").unwrap();
    let output = doctor::run_with_options(
        vec![McpClient::Opencode],
        Some(dir.path().to_owned()),
        Some(config),
        true,
        true,
        None,
    )
    .await
    .unwrap();
    assert!(output.contains("MCP probe failed"));
    assert!(
        output.contains("no OpenCode LazyDB server is configured"),
        "{output}"
    );
    assert!(output.contains("\"status\":\"failed\""));
}
