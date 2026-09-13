use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

use tempfile::TempDir;

fn empty_profile_file(temp: &TempDir) -> std::path::PathBuf {
    let path = temp.path().join("profiles.toml");
    std::fs::write(&path, "version = 6\nprofiles = []\n").unwrap();
    path
}

fn request(child: &mut std::process::Child, request: serde_json::Value) -> serde_json::Value {
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, "{}", request).unwrap();
    stdin.flush().unwrap();

    let stdout = child.stdout.as_mut().unwrap();
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn stdio_server_negotiates_and_lists_tools_without_database_io() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();
    let profiles = empty_profile_file(&temp);
    let mut child = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .args([
            "--config",
            profiles.to_str().unwrap(),
            "mcp",
            "serve",
            "--project",
            temp.path().to_str().unwrap(),
            "--write-policy",
            "deny",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let initialized = request(
        &mut child,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "lazydb-test-client", "version": "1"}
            }
        }),
    );
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(initialized["result"]["serverInfo"]["name"], "lazydb");
    assert_eq!(
        initialized["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        initialized["result"]["capabilities"]["tools"],
        serde_json::json!({})
    );

    writeln!(
        child.stdin.as_mut().unwrap(),
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        })
    )
    .unwrap();
    child.stdin.as_mut().unwrap().flush().unwrap();

    let listed = request(
        &mut child,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    let mut names = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "describe_object",
            "execute_change",
            "execute_file",
            "get_context",
            "list_connections",
            "query",
            "search_schema",
        ]
    );

    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unsupported_profile_version_is_reported_before_mcp_handshake() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();
    let profiles = temp.path().join("profiles.toml");
    std::fs::write(&profiles, "version = 1\nprofiles = []\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .args([
            "--config",
            profiles.to_str().unwrap(),
            "mcp",
            "serve",
            "--project",
            temp.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("profile file version 1 is not supported"),
        "stderr: {stderr}"
    );
}
