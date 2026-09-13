use std::time::Duration;

#[tokio::test]
async fn probes_current_lazydb_without_database_io() {
    let temp = tempfile::tempdir().unwrap();
    let profiles = temp.path().join("profiles.toml");
    std::fs::write(&profiles, "version = 6\nprofiles = []\n").unwrap();
    let command = vec![
        env!("CARGO_BIN_EXE_lazydb").to_owned(),
        "--config".into(),
        profiles.to_string_lossy().into_owned(),
        "mcp".into(),
        "serve".into(),
        "--project".into(),
        temp.path().to_string_lossy().into_owned(),
        "--write-policy".into(),
        "deny".into(),
    ];
    let report = lazydb::agent::probe::run(
        &command,
        Some(temp.path()),
        &[],
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(report.status, "ok");
    assert_eq!(report.stage, "tools/list");
    assert_eq!(report.tool_count, Some(7));
}

#[tokio::test]
async fn reports_failed_server_start() {
    let error = lazydb::agent::probe::run(
        &["definitely-not-a-server".into()],
        None,
        &[],
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("failed to start MCP server"), "{error}");
}
