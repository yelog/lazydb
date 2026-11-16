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
            "get_data_task",
            "list_connections",
            "list_data_tasks",
            "mongo_describe_collection",
            "mongo_execute_change",
            "mongo_list_collections",
            "mongo_query",
            "plan_data_operation",
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
fn stdio_server_queries_a_clickhouse_profile_end_to_end() {
    use std::{io::Read, net::TcpListener, thread};

    let fixture = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = fixture.local_addr().unwrap();
    let clickhouse = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = fixture.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
                let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let header_end = header_end + 4;
                let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + content_length {
                    break;
                }
            }
            let raw = String::from_utf8_lossy(&request);
            let header_end = raw.find("\r\n\r\n").unwrap() + 4;
            let sql = &raw[header_end..];
            let body = if sql.contains("SELECT version()") {
                "version()\tcurrentDatabase()\tcurrentUser()\nString\tString\tString\n25.8.1\tanalytics\tdefault\n"
            } else {
                "answer\nUInt64\n42\n"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let temp = TempDir::new().unwrap();
    std::fs::create_dir(temp.path().join(".git")).unwrap();
    let profiles = temp.path().join("profiles.toml");
    let project_root = temp.path().canonicalize().unwrap();
    let mut profile = lazydb::profile::import_connection_url(
        &format!("clickhouse://127.0.0.1:{}/analytics", address.port()),
        Some("ch-mcp"),
    )
    .unwrap()
    .profile;
    profile.access = lazydb::profile::ProfileAccess::Projects {
        roots: vec![project_root],
    };
    lazydb::persistence::profiles::ProfileStore::new(profiles.clone())
        .save(vec![profile])
        .unwrap();
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
            "jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                "protocolVersion":"2025-06-18", "capabilities":{},
                "clientInfo":{"name":"lazydb-clickhouse-test", "version":"1"}
            }
        }),
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "lazydb");
    writeln!(
        child.stdin.as_mut().unwrap(),
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0", "method":"notifications/initialized"
        })
    )
    .unwrap();
    child.stdin.as_mut().unwrap().flush().unwrap();
    let result = request(
        &mut child,
        serde_json::json!({
            "jsonrpc":"2.0", "id":2, "method":"tools/call", "params":{
                "name":"query", "arguments":{"connection":"ch-mcp", "sql":"SELECT 42 AS answer"}
            }
        }),
    );
    let text = result["result"]["content"][0]["text"].as_str().unwrap();
    let value: serde_json::Value = if text.starts_with('{') {
        serde_json::from_str(text).unwrap()
    } else {
        panic!("unexpected MCP response: {result}")
    };
    assert_eq!(value["target"]["connection"]["kind"], "clickhouse");
    assert_eq!(
        value["outcome"]["result_sets"][0]["rows"][0][0]["Unsigned"],
        42
    );
    assert!(text.contains("ch-mcp"));
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    clickhouse.join().unwrap();
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
