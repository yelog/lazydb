#![cfg(unix)]

#[path = "fixtures/ssh/test_support.rs"]
mod support;

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio},
    sync::{Mutex, MutexGuard, mpsc},
    thread,
    time::{Duration, Instant},
};

use lazydb::{
    persistence::{local_credentials::LocalCredentialStore, profiles::ProfileStore},
    profile::{ConnectionProfile, CredentialPolicy, PgConnectionOptions, SshOptions, SslMode},
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::{FixtureManifest, POSTGRES_PASSWORD_ENV, fixture, required_env};
use tempfile::TempDir;

const PROFILE_NAME: &str = "ssh-postgres-services";
const AGENT_MARKER: &str = "ssh-agent-ok";
const MCP_MARKER: &str = "ssh-mcp-ok";
const CHILD_TIMEOUT: Duration = Duration::from_secs(45);
const EXIT_TIMEOUT: Duration = Duration::from_secs(10);

static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

fn fixture_lock() -> MutexGuard<'static, ()> {
    FIXTURE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct TestEnvironment {
    _root: TempDir,
    config_dir: PathBuf,
    profiles: PathBuf,
    project: PathBuf,
    ssh_config: PathBuf,
    password: SecretString,
}

impl TestEnvironment {
    fn new(fixture: &FixtureManifest) -> Self {
        let root = tempfile::tempdir().expect("create SSH service test directory");
        let config_dir = root.path().join("config");
        let project = root.path().join("project");
        std::fs::create_dir_all(&config_dir).expect("create isolated config home");
        std::fs::create_dir_all(project.join(".git")).expect("create project marker");
        std::fs::write(
            config_dir.join("settings.toml"),
            "[updates]\ncheck_on_startup = false\n",
        )
        .expect("write isolated settings");
        let ssh_config = root.path().join("ssh_config");
        std::os::unix::fs::symlink(&fixture.ssh.config_file, &ssh_config)
            .expect("create isolated SSH config link");

        let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
        assert!(
            !password.expose_secret().is_empty(),
            "the PostgreSQL fixture password must not be empty"
        );
        let url = format!(
            "postgresql://{}@{}:{}/{}",
            fixture.postgres.user,
            fixture.postgres.host,
            fixture.postgres.port,
            fixture.postgres.database
        );
        let mut profile: ConnectionProfile =
            lazydb::profile::import_connection_url(&url, Some(PROFILE_NAME))
                .expect("import PostgreSQL fixture profile")
                .profile;
        profile.ssl_mode = SslMode::VerifyFull;
        profile.pg_options = Some(PgConnectionOptions {
            ssl_root_cert: Some(fixture.postgres.ca_file.clone()),
            ssl_client_cert: Some(fixture.postgres.client_cert.clone()),
            ssl_client_key: Some(fixture.postgres.client_key.clone()),
            options: None,
        });
        profile.ssh = Some(SshOptions {
            host: fixture.ssh.host.clone(),
            port: Some(fixture.ssh.port),
            user: Some(fixture.ssh.user.clone()),
            identity_file: Some(fixture.ssh.client_key.clone()),
            config_file: Some(ssh_config.clone()),
            known_hosts_file: Some(fixture.ssh.known_hosts_file.clone()),
            proxy_jump: None,
        });
        profile.read_only = true;

        let credential_key = config_dir.join("credential.key");
        let local_store = LocalCredentialStore::new(credential_key.clone(), "lazydb");
        profile.credential_policy = CredentialPolicy::LocalEncrypted(
            local_store
                .encrypt(profile.id, &password)
                .expect("encrypt fixture password for the isolated profile"),
        );
        let profiles = config_dir.join("connections.toml");
        ProfileStore::new(profiles.clone())
            .with_credential_key_path(credential_key)
            .save(vec![profile])
            .expect("save isolated SSH profile");

        let contents = std::fs::read(&profiles).expect("read isolated profile");
        assert!(
            !contains_secret(&contents, &password),
            "the fixture password was persisted in profile metadata"
        );
        Self {
            _root: root,
            config_dir,
            profiles,
            project,
            ssh_config,
            password,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazydb"));
        command
            .env_remove("LAZYDB_PASSWORD")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("SSL_CERT_FILE")
            .env("LAZYDB_CONFIG_HOME", &self.config_dir)
            .current_dir(&self.project);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("LAZYDB_TEST_SSH_") {
                command.env_remove(key);
            }
        }
        command
    }

    fn assert_no_secret(&self, bytes: &[u8], context: &str) {
        assert!(
            !contains_secret(bytes, &self.password),
            "{context} exposed the fixture password"
        );
    }

    fn redacted(&self, bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace(self.password.expose_secret(), "[REDACTED]")
    }

    fn parse_success_json(&self, output: &CommandOutput, context: &str) -> Value {
        self.assert_no_secret(&output.stdout, context);
        self.assert_no_secret(&output.stderr, context);
        assert!(
            output.status.success(),
            "{context} exited with {}\n{}",
            output.status,
            self.redacted(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "{context} wrote to stderr:\n{}",
            self.redacted(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("{context} stdout must contain exactly one JSON document: {error}")
        })
    }
}

struct CommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_command(mut command: Command, timeout: Duration) -> CommandOutput {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start lazydb acceptance process");
    let status = match wait_for_exit(&mut child, timeout) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("lazydb acceptance process exceeded {timeout:?}");
        }
    };
    let stdout = read_pipe(child.stdout.take());
    let stderr = read_pipe(child.stderr.take());
    CommandOutput {
        status,
        stdout,
        stderr,
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("poll acceptance process") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn read_pipe<T: Read>(pipe: Option<T>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes)
            .expect("read acceptance process output");
    }
    bytes
}

fn contains_secret(bytes: &[u8], password: &SecretString) -> bool {
    let secret = password.expose_secret().as_bytes();
    bytes
        .windows(secret.len())
        .any(|candidate| candidate == secret)
}

struct SshProcessGuard {
    ssh_config: PathBuf,
}

impl SshProcessGuard {
    fn new(ssh_config: &Path) -> Self {
        Self {
            ssh_config: ssh_config.to_path_buf(),
        }
    }

    fn wait_until_stopped(&self) {
        let deadline = Instant::now() + EXIT_TIMEOUT;
        loop {
            let pids = managed_ssh_pids(&self.ssh_config)
                .unwrap_or_else(|error| panic!("unable to verify SSH cleanup: {error}"));
            if pids.is_empty() {
                return;
            }
            if Instant::now() >= deadline {
                kill_pids(&pids);
                panic!("managed SSH tunnel processes were not reaped: {pids:?}");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn kill_managed(&self) {
        let Ok(mut pids) = managed_ssh_pids(&self.ssh_config) else {
            return;
        };
        if pids.is_empty() {
            return;
        }
        kill_pids(&pids);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !pids.is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
            let Ok(current) = managed_ssh_pids(&self.ssh_config) else {
                return;
            };
            pids = current;
        }
    }
}

impl Drop for SshProcessGuard {
    fn drop(&mut self) {
        self.kill_managed();
    }
}

fn managed_ssh_pids(ssh_config: &Path) -> Result<Vec<u32>, String> {
    let output = Command::new("ps")
        .args(["-ax", "-ww", "-o", "pid=", "-o", "command="])
        .output()
        .map_err(|error| format!("unable to read process table: {error}"))?;
    if !output.status.success() {
        return Err(format!("ps exited with {}", output.status));
    }
    let expected_config = ssh_config.display().to_string();
    let mut pids = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim_start();
        let Some(separator) = trimmed.find(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = trimmed[..separator].parse::<u32>() else {
            continue;
        };
        let arguments = trimmed[separator..]
            .split_ascii_whitespace()
            .collect::<Vec<_>>();
        let targets_config = arguments
            .windows(2)
            .any(|pair| pair[0] == "-F" && pair[1] == expected_config.as_str());
        let batch_mode = arguments
            .windows(2)
            .any(|pair| pair[0] == "-o" && pair[1] == "BatchMode=yes");
        let has_forward = arguments
            .iter()
            .any(|argument| matches!(*argument, "-L" | "-D"));
        if targets_config && batch_mode && has_forward {
            pids.push(pid);
        }
    }
    Ok(pids)
}

fn ensure_managed_ssh_running(ssh_config: &Path) -> Result<(), String> {
    if managed_ssh_pids(ssh_config)?.is_empty() {
        return Err("profile catalog did not keep its SSH tunnel process alive".to_owned());
    }
    Ok(())
}

fn kill_pids(pids: &[u32]) {
    for pid in pids {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }
}

struct ProtocolOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exchange_error: Option<String>,
}

fn run_protocol_process<F>(mut command: Command, timeout: Duration, exchange: F) -> ProtocolOutput
where
    F: FnOnce(ChildStdin, ChildStdout) -> (Vec<u8>, Result<(), String>) + Send + 'static,
{
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start lazydb protocol process");
    let input = child.stdin.take().expect("protocol stdin");
    let output = child.stdout.take().expect("protocol stdout");
    let error = child.stderr.take().expect("protocol stderr");
    let stderr_thread = thread::spawn(move || read_pipe(Some(error)));
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = exchange(input, output);
        let _ = sender.send(result);
    });

    let mut exchange_error = None;
    let (stdout, result) = match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let result = receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|_| {
                    (
                        Vec::new(),
                        Err("protocol exchange did not stop after termination".to_owned()),
                    )
                });
            exchange_error = Some("protocol exchange timed out".to_owned());
            result
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            (
                Vec::new(),
                Err("protocol worker stopped without a result".to_owned()),
            )
        }
    };
    if let Err(error) = result {
        exchange_error = Some(error);
    }
    let status = match wait_for_exit(&mut child, EXIT_TIMEOUT) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let status = child.wait().expect("reap terminated protocol process");
            exchange_error.get_or_insert_with(|| {
                "protocol server did not exit within the shutdown deadline".to_owned()
            });
            status
        }
    };
    let _ = worker.join();
    let stderr = stderr_thread.join().unwrap_or_default();
    ProtocolOutput {
        status,
        stdout,
        stderr,
        exchange_error,
    }
}

fn write_mcp_json(input: &mut ChildStdin, message: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *input, message)
        .map_err(|_| "unable to serialize MCP request".to_owned())?;
    input
        .write_all(b"\n")
        .and_then(|_| input.flush())
        .map_err(|_| "unable to write MCP request".to_owned())
}

fn read_mcp_message(
    reader: &mut BufReader<ChildStdout>,
    raw: &mut Vec<u8>,
) -> Result<Option<Value>, String> {
    let mut line = Vec::new();
    let read = reader
        .read_until(b'\n', &mut line)
        .map_err(|_| "unable to read MCP stdout".to_owned())?;
    if read == 0 {
        return Ok(None);
    }
    raw.extend_from_slice(&line);
    let text = std::str::from_utf8(&line)
        .map_err(|_| "MCP stdout was not UTF-8 JSON".to_owned())?
        .trim_end_matches(['\r', '\n']);
    if text.is_empty() {
        return Err("MCP stdout contained an empty line".to_owned());
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|_| "MCP stdout contained a non-JSON line".to_owned())?;
    if value.get("jsonrpc") != Some(&json!("2.0")) {
        return Err("MCP stdout message was not JSON-RPC 2.0".to_owned());
    }
    Ok(Some(value))
}

fn read_mcp_response(
    reader: &mut BufReader<ChildStdout>,
    raw: &mut Vec<u8>,
    id: u64,
) -> Result<Value, String> {
    while let Some(message) = read_mcp_message(reader, raw)? {
        if message.get("id") == Some(&json!(id)) {
            return Ok(message);
        }
    }
    Err(format!("MCP server closed stdout before response {id}"))
}

fn drain_mcp(reader: &mut BufReader<ChildStdout>, raw: &mut Vec<u8>) -> Result<(), String> {
    while read_mcp_message(reader, raw)?.is_some() {}
    Ok(())
}

fn mcp_script(
    input: &mut ChildStdin,
    reader: &mut BufReader<ChildStdout>,
    raw: &mut Vec<u8>,
) -> Result<(), String> {
    write_mcp_json(
        input,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "lazydb-ssh-services", "version": "1.0"}
            }
        }),
    )?;
    let initialized = read_mcp_response(reader, raw, 1)?;
    if initialized
        .pointer("/result/serverInfo/name")
        .and_then(Value::as_str)
        != Some("lazydb")
    {
        return Err("MCP initialize did not identify the LazyDB server".to_owned());
    }
    write_mcp_json(
        input,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )?;
    write_mcp_json(
        input,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )?;
    let tools = read_mcp_response(reader, raw, 2)?;
    let has_query = tools
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "query"));
    if !has_query {
        return Err("MCP tools/list did not include query".to_owned());
    }
    let sql = format!("SELECT '{MCP_MARKER}'::text AS marker");
    write_mcp_json(
        input,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "query",
                "arguments": {"connection": PROFILE_NAME, "sql": sql}
            }
        }),
    )?;
    let result = read_mcp_response(reader, raw, 3)?;
    if result.pointer("/result/isError").and_then(Value::as_bool) == Some(true) {
        return Err("MCP query returned an error result".to_owned());
    }
    let text = result
        .pointer("/result/content")
        .and_then(Value::as_array)
        .and_then(|content| {
            content.iter().find_map(|entry| {
                (entry["type"] == "text")
                    .then(|| entry["text"].as_str())
                    .flatten()
            })
        })
        .ok_or_else(|| "MCP query result contained no text content".to_owned())?;
    let payload: Value =
        serde_json::from_str(text).map_err(|_| "MCP query text content was not JSON".to_owned())?;
    if payload
        .pointer("/outcome/result_sets/0/rows/0/0/Text")
        .and_then(Value::as_str)
        != Some(MCP_MARKER)
    {
        return Err("MCP query did not return the expected PostgreSQL marker".to_owned());
    }
    Ok(())
}

fn mcp_exchange(input: ChildStdin, output: ChildStdout) -> (Vec<u8>, Result<(), String>) {
    let mut input = input;
    let mut reader = BufReader::new(output);
    let mut raw = Vec::new();
    let mut result = mcp_script(&mut input, &mut reader, &mut raw);
    if result.is_ok() {
        drop(input);
        result = drain_mcp(&mut reader, &mut raw);
    }
    (raw, result)
}

struct LspReader {
    reader: BufReader<ChildStdout>,
    raw: Vec<u8>,
}

impl LspReader {
    fn new(output: ChildStdout) -> Self {
        Self {
            reader: BufReader::new(output),
            raw: Vec::new(),
        }
    }

    fn read_message(&mut self) -> Result<Option<Value>, String> {
        let mut content_length = None;
        loop {
            let mut line = Vec::new();
            let read = self
                .reader
                .read_until(b'\n', &mut line)
                .map_err(|_| "unable to read LSP stdout".to_owned())?;
            if read == 0 {
                if content_length.is_some() {
                    return Err("LSP stdout ended inside a message body".to_owned());
                }
                return Ok(None);
            }
            self.raw.extend_from_slice(&line);
            let header = std::str::from_utf8(&line)
                .map_err(|_| "LSP stdout header was not UTF-8".to_owned())?
                .strip_suffix("\r\n")
                .ok_or_else(|| "LSP stdout header did not use CRLF".to_owned())?;
            if header.is_empty() {
                break;
            }
            let (name, value) = header
                .split_once(':')
                .ok_or_else(|| "LSP stdout contained an unframed line".to_owned())?;
            if name.eq_ignore_ascii_case("content-length") {
                if content_length.is_some() {
                    return Err("LSP stdout repeated Content-Length".to_owned());
                }
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| "LSP Content-Length was invalid".to_owned())?,
                );
            } else if !name.eq_ignore_ascii_case("content-type") {
                return Err("LSP stdout contained an unexpected header".to_owned());
            }
        }
        let length =
            content_length.ok_or_else(|| "LSP message omitted Content-Length".to_owned())?;
        let mut body = vec![0; length];
        self.reader
            .read_exact(&mut body)
            .map_err(|_| "unable to read the complete LSP message".to_owned())?;
        self.raw.extend_from_slice(&body);
        let message: Value =
            serde_json::from_slice(&body).map_err(|_| "LSP stdout body was not JSON".to_owned())?;
        if message.get("jsonrpc") != Some(&json!("2.0")) {
            return Err("LSP stdout message was not JSON-RPC 2.0".to_owned());
        }
        Ok(Some(message))
    }

    fn read_response(&mut self, id: u64) -> Result<Value, String> {
        while let Some(message) = self.read_message()? {
            if message.get("id") == Some(&json!(id)) {
                return Ok(message);
            }
        }
        Err(format!("LSP server closed stdout before response {id}"))
    }
}

fn write_lsp(input: &mut ChildStdin, message: &Value) -> Result<(), String> {
    let body =
        serde_json::to_vec(message).map_err(|_| "unable to serialize LSP message".to_owned())?;
    write!(input, "Content-Length: {}\r\n\r\n", body.len())
        .and_then(|_| input.write_all(&body))
        .and_then(|_| input.flush())
        .map_err(|_| "unable to write LSP message".to_owned())
}

fn lsp_script(
    input: &mut ChildStdin,
    reader: &mut LspReader,
    ssh_config: &Path,
) -> Result<(), String> {
    write_lsp(
        input,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"processId": null, "rootUri": null, "capabilities": {}}
        }),
    )?;
    let initialized = reader.read_response(1)?;
    if initialized
        .pointer("/result/capabilities/textDocumentSync")
        .and_then(Value::as_u64)
        != Some(1)
    {
        return Err("LSP initialize did not advertise full document sync".to_owned());
    }
    ensure_managed_ssh_running(ssh_config)?;
    write_lsp(
        input,
        &json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
    )?;
    write_lsp(
        input,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null}),
    )?;
    let shutdown = reader.read_response(2)?;
    if shutdown.get("error").is_some() {
        return Err("LSP shutdown returned an error".to_owned());
    }
    write_lsp(
        input,
        &json!({"jsonrpc": "2.0", "method": "exit", "params": null}),
    )?;
    while reader.read_message()?.is_some() {}
    Ok(())
}

fn lsp_exchange(
    input: ChildStdin,
    output: ChildStdout,
    ssh_config: &Path,
) -> (Vec<u8>, Result<(), String>) {
    let mut input = input;
    let mut reader = LspReader::new(output);
    let result = lsp_script(&mut input, &mut reader, ssh_config);
    (reader.raw, result)
}

#[test]
#[ignore = "requires the isolated OpenSSH and PostgreSQL TLS fixture"]
fn connections_test_probes_postgres_through_ssh() {
    let _serial = fixture_lock();
    let fixture = fixture();
    let environment = TestEnvironment::new(&fixture);
    let ssh = SshProcessGuard::new(&environment.ssh_config);
    let mut command = environment.command();
    command
        .arg("--config")
        .arg(&environment.profiles)
        .args(["connections", "test", PROFILE_NAME, "--project"])
        .arg(&environment.project)
        .args(["--timeout", "20", "--json"]);
    let output = run_command(command, CHILD_TIMEOUT);
    let value = environment.parse_success_json(&output, "connections test");

    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "connections.test");
    assert_eq!(value["data"]["connection"]["name"], PROFILE_NAME);
    assert_eq!(
        value["data"]["connection"]["ssh"]["host"],
        fixture.ssh.host.as_str()
    );
    assert_eq!(
        value["data"]["server"]["database"],
        fixture.postgres.database.as_str()
    );
    ssh.wait_until_stopped();
}

#[test]
#[ignore = "requires the isolated OpenSSH and PostgreSQL TLS fixture"]
fn agent_query_runs_postgres_through_ssh() {
    let _serial = fixture_lock();
    let fixture = fixture();
    let environment = TestEnvironment::new(&fixture);
    let ssh = SshProcessGuard::new(&environment.ssh_config);
    let sql = format!("SELECT '{AGENT_MARKER}'::text AS marker");
    let mut command = environment.command();
    command
        .arg("--config")
        .arg(&environment.profiles)
        .args(["agent", "query", "--project"])
        .arg(&environment.project)
        .args(["--connection", PROFILE_NAME, "--sql", &sql]);
    let output = run_command(command, CHILD_TIMEOUT);
    let value = environment.parse_success_json(&output, "agent query");

    assert_eq!(value["target"]["connection"]["name"], PROFILE_NAME);
    assert_eq!(value["outcome"]["row_count"], 1);
    assert_eq!(
        value["outcome"]["result_sets"][0]["rows"][0][0]["Text"],
        AGENT_MARKER
    );
    ssh.wait_until_stopped();
}

#[test]
#[ignore = "requires the isolated OpenSSH and PostgreSQL TLS fixture"]
fn mcp_lists_tools_queries_postgres_and_exits_cleanly_on_eof() {
    let _serial = fixture_lock();
    let fixture = fixture();
    let environment = TestEnvironment::new(&fixture);
    let ssh = SshProcessGuard::new(&environment.ssh_config);
    let mut command = environment.command();
    command
        .arg("--config")
        .arg(&environment.profiles)
        .args(["mcp", "serve", "--project"])
        .arg(&environment.project)
        .args(["--connection", PROFILE_NAME, "--write-policy", "deny"]);
    let output = run_protocol_process(command, CHILD_TIMEOUT, mcp_exchange);

    environment.assert_no_secret(&output.stdout, "MCP stdout");
    environment.assert_no_secret(&output.stderr, "MCP stderr");
    if let Some(error) = output.exchange_error {
        panic!(
            "MCP exchange failed: {error}\n{}",
            environment.redacted(&output.stderr)
        );
    }
    assert!(
        output.status.success(),
        "MCP server exited with {}\n{}",
        output.status,
        environment.redacted(&output.stderr)
    );
    ssh.wait_until_stopped();
}

#[test]
#[ignore = "requires the isolated OpenSSH and PostgreSQL TLS fixture"]
fn lsp_initializes_profile_catalog_keeps_stdout_framed_and_shuts_down() {
    let _serial = fixture_lock();
    let fixture = fixture();
    let environment = TestEnvironment::new(&fixture);
    let ssh = SshProcessGuard::new(&environment.ssh_config);
    let mut command = environment.command();
    command
        .arg("--config")
        .arg(&environment.profiles)
        .args(["--profile", PROFILE_NAME, "lsp", "--stdio", "--project"])
        .arg(&environment.project);
    let ssh_config = environment.ssh_config.clone();
    let output = run_protocol_process(command, CHILD_TIMEOUT, move |input, output| {
        lsp_exchange(input, output, &ssh_config)
    });

    environment.assert_no_secret(&output.stdout, "LSP stdout");
    environment.assert_no_secret(&output.stderr, "LSP stderr");
    if let Some(error) = output.exchange_error {
        panic!(
            "LSP exchange failed: {error}\n{}",
            environment.redacted(&output.stderr)
        );
    }
    assert!(
        output.status.success(),
        "LSP server exited with {}\n{}",
        output.status,
        environment.redacted(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("catalog provider initialized"),
        "LSP did not initialize the saved profile catalog"
    );
    assert!(
        !stderr.contains("catalog unavailable"),
        "LSP fell back to offline completion"
    );
    ssh.wait_until_stopped();
}
