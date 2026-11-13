#![allow(dead_code)]

use std::{
    env,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command as StdCommand,
    process::Stdio,
    time::{Duration, Instant},
};

use serde::Deserialize;
use tempfile::TempDir;
use tokio::process::{Child, Command};

pub const MANIFEST_ENV: &str = "LAZYDB_TEST_SSH_FIXTURE_MANIFEST";
pub const POSTGRES_PASSWORD_ENV: &str = "LAZYDB_TEST_SSH_POSTGRES_PASSWORD";
pub const MYSQL_PASSWORD_ENV: &str = "LAZYDB_TEST_SSH_MYSQL_PASSWORD";
pub const REDIS_PASSWORD_ENV: &str = "LAZYDB_TEST_SSH_REDIS_PASSWORD";
pub const MONGO_USERNAME_ENV: &str = "LAZYDB_TEST_SSH_MONGO_USERNAME";
pub const MONGO_PASSWORD_ENV: &str = "LAZYDB_TEST_SSH_MONGO_PASSWORD";

#[derive(Clone, Debug, Deserialize)]
pub struct FixtureManifest {
    pub schema_version: u32,
    pub ssh: SshFixture,
    pub jump: JumpFixture,
    pub http: HttpFixture,
    pub mongo: MongoFixture,
    pub postgres: PostgresFixture,
    pub mysql: MySqlFixture,
    pub redis: RedisFixture,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SshFixture {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub client_key: PathBuf,
    pub wrong_client_key: PathBuf,
    pub config_file: PathBuf,
    pub known_hosts_file: PathBuf,
    pub wrong_known_hosts_file: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JumpFixture {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub final_host: String,
    pub final_port: u16,
    pub proxy_alias: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HttpFixture {
    pub clickhouse_url: String,
    pub elasticsearch_url: String,
    pub opensearch_url: String,
    pub ca_file: PathBuf,
    pub wrong_ca_file: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MongoFixture {
    pub url: String,
    pub ca_file: PathBuf,
    pub database: String,
    pub replica_set: String,
    #[serde(default)]
    pub members: Vec<MongoMember>,
    #[serde(default)]
    pub username_env: Option<String>,
    #[serde(default)]
    pub password_env: Option<String>,
    #[serde(default)]
    pub auth_source: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MongoMember {
    pub host: String,
    pub port: u16,
}

impl MongoFixture {
    pub fn username(&self) -> String {
        required_env(self.username_env.as_deref().unwrap_or(MONGO_USERNAME_ENV))
    }

    pub fn password(&self) -> String {
        required_env(self.password_env.as_deref().unwrap_or(MONGO_PASSWORD_ENV))
    }

    pub fn auth_source(&self) -> &str {
        self.auth_source.as_deref().unwrap_or("admin")
    }

    pub fn require_members(&self) -> &[MongoMember] {
        assert!(
            !self.members.is_empty(),
            "SSH fixture manifest does not contain MongoDB replica-set members"
        );
        &self.members
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PostgresFixture {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub ca_file: PathBuf,
    pub wrong_ca_file: PathBuf,
    pub client_cert: PathBuf,
    pub client_key: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MySqlFixture {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub ca_file: PathBuf,
    pub wrong_ca_file: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RedisFixture {
    pub host: String,
    pub port: u16,
    pub database: u16,
    pub second_database: u16,
}

pub fn fixture() -> FixtureManifest {
    let path = env::var(MANIFEST_ENV).unwrap_or_else(|_| {
        panic!("{MANIFEST_ENV} is required; run scripts/ci/test-ssh-integration.py")
    });
    let contents = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("unable to read SSH fixture manifest {path}: {error}"));
    let manifest: FixtureManifest = serde_json::from_slice(&contents)
        .unwrap_or_else(|error| panic!("invalid SSH fixture manifest {path}: {error}"));
    assert_eq!(
        manifest.schema_version, 1,
        "unsupported SSH fixture manifest version"
    );
    manifest
}

pub fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} is required for the SSH fixture"))
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ForwardKind {
    Unix,
    Tcp,
    Socks,
}

pub struct SshForwarder {
    _log_directory: TempDir,
    child: Child,
    log_path: PathBuf,
    unix_path: Option<PathBuf>,
    tcp_addr: Option<SocketAddr>,
    socks_addr: Option<SocketAddr>,
    kind: ForwardKind,
    marker: String,
    listener_type: &'static str,
}

impl SshForwarder {
    pub async fn start_unix(
        ssh: &SshFixture,
        socket_path: &Path,
        remote_host: &str,
        remote_port: u16,
    ) -> Self {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).expect("create Unix socket directory");
        }
        let target = format!(
            "{}:{}:{}",
            socket_path.display(),
            bracket_host(remote_host),
            remote_port
        );
        let directory = fixture_tempdir();
        let mut forwarder = Self::spawn(
            ssh,
            "-L",
            &target,
            directory,
            ForwardKind::Unix,
            format!("path {}", socket_path.display()),
            "new unix-listener",
        )
        .await;
        forwarder.unix_path = Some(socket_path.to_path_buf());
        forwarder.wait_ready().await;
        forwarder
    }

    pub async fn start_tcp(ssh: &SshFixture, remote_host: &str, remote_port: u16) -> Self {
        let directory = fixture_tempdir();
        let local_port = reserve_loopback_port();
        let target = format!(
            "127.0.0.1:{local_port}:{}:{remote_port}",
            bracket_host(remote_host)
        );
        let marker = format!("127.0.0.1 port {local_port}");
        let mut forwarder = Self::spawn(
            ssh,
            "-L",
            &target,
            directory,
            ForwardKind::Tcp,
            marker,
            "new port-listener",
        )
        .await;
        forwarder.tcp_addr = Some(
            format!("127.0.0.1:{local_port}")
                .parse()
                .expect("valid forwarded socket address"),
        );
        forwarder.wait_ready().await;
        forwarder
    }

    pub async fn start_socks(ssh: &SshFixture) -> Self {
        let directory = fixture_tempdir();
        let local_port = reserve_loopback_port();
        let marker = format!("127.0.0.1 port {local_port}");
        let mut forwarder = Self::spawn(
            ssh,
            "-D",
            &format!("127.0.0.1:{local_port}"),
            directory,
            ForwardKind::Socks,
            marker,
            "new port-listener",
        )
        .await;
        forwarder.socks_addr = Some(
            format!("127.0.0.1:{local_port}")
                .parse()
                .expect("valid SOCKS socket address"),
        );
        forwarder.wait_ready().await;
        forwarder
    }

    async fn spawn(
        ssh: &SshFixture,
        forward_flag: &str,
        forward: &str,
        directory: TempDir,
        kind: ForwardKind,
        marker: String,
        listener_type: &'static str,
    ) -> Self {
        let log_path = directory.path().join("ssh.log");
        let mut command = Command::new("ssh");
        command
            .arg("-F")
            .arg(&ssh.config_file)
            .arg("-N")
            .arg("-T")
            .arg("-i")
            .arg(&ssh.client_key)
            .arg("-p")
            .arg(ssh.port.to_string())
            .arg("-E")
            .arg(&log_path)
            .arg("-o")
            .arg("ExitOnForwardFailure=yes")
            .arg("-o")
            .arg("LogLevel=DEBUG1");
        if kind == ForwardKind::Unix {
            command
                .arg("-o")
                .arg("StreamLocalBindMask=0177")
                .arg("-o")
                .arg("StreamLocalBindUnlink=no");
        }
        let child = command
            .arg(forward_flag)
            .arg(forward)
            .arg(format!("{}@{}", ssh.user, ssh.host))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn OpenSSH");
        Self {
            _log_directory: directory,
            child,
            log_path,
            unix_path: None,
            tcp_addr: None,
            socks_addr: None,
            kind,
            marker,
            listener_type,
        }
    }

    async fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().expect("poll ssh process") {
                panic!(
                    "OpenSSH exited before forwarding became ready ({status}):\n{}",
                    tail(&self.log())
                );
            }
            let listener_ready = match self.kind {
                ForwardKind::Unix => self.unix_path.as_ref().is_some_and(|path| is_socket(path)),
                ForwardKind::Tcp | ForwardKind::Socks => true,
            };
            if listener_ready && self.marker_positions().is_some() {
                return;
            }
            if Instant::now() >= deadline {
                let _ = self.child.start_kill();
                panic!(
                    "OpenSSH forwarding did not become ready:\n{}",
                    tail(&self.log())
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub fn unix_path(&self) -> &PathBuf {
        self.unix_path
            .as_ref()
            .expect("Unix forwarding was not requested")
    }

    pub fn tcp_addr(&self) -> SocketAddr {
        self.tcp_addr.expect("TCP forwarding was not requested")
    }

    pub fn socks_addr(&self) -> SocketAddr {
        self.socks_addr.expect("SOCKS forwarding was not requested")
    }

    pub fn assert_ready_marker_order(&self) {
        let log = self.log();
        let lines: Vec<&str> = log.lines().collect();
        let positions = self
            .marker_positions()
            .unwrap_or_else(|| panic!("missing OpenSSH readiness markers:\n{}", tail(&log)));
        assert!(
            positions.0 < positions.1 && positions.1 < positions.2,
            "OpenSSH readiness markers are out of order:\n{}\n{}",
            lines[positions.0],
            lines[positions.1]
        );
        assert!(
            lines[positions.2].trim_end_matches('\r') == "debug1: Entering interactive session.",
            "OpenSSH session marker was not completed:\n{}",
            lines[positions.2]
        );
    }

    fn marker_positions(&self) -> Option<(usize, usize, usize)> {
        let log = self.log();
        let lines: Vec<&str> = log.lines().collect();
        let expected_local = format!("debug1: Local forwarding listening on {}.", self.marker);
        let local = lines
            .iter()
            .position(|line| line.trim_end_matches('\r').starts_with(&expected_local))?;
        let listener = lines
            .iter()
            .enumerate()
            .skip(local + 1)
            .find(|(_, line)| {
                line.starts_with("debug1: channel ") && line.contains(self.listener_type)
            })
            .map(|(position, _)| position)?;
        let session = lines
            .iter()
            .enumerate()
            .skip(listener + 1)
            .find(|(_, line)| {
                line.trim_end_matches('\r') == "debug1: Entering interactive session."
            })
            .map(|(position, _)| position)?;
        Some((local, listener, session))
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    pub async fn shutdown(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

pub fn fixture_tempdir() -> TempDir {
    tempfile::Builder::new()
        .prefix("ldbssh-")
        .tempdir_in("/tmp")
        .expect("create short Unix-socket fixture directory")
}

#[derive(Clone, Debug)]
pub struct ManagedSshTunnel {
    pub pid: u32,
    pub local_port: Option<u16>,
}

pub fn kill_managed_ssh_tunnels(identity_key: &Path) -> Vec<ManagedSshTunnel> {
    let output = StdCommand::new("ps")
        .args(["-ax", "-o", "pid=", "-o", "command="])
        .output()
        .expect("read process table");
    assert!(output.status.success(), "ps failed: {:?}", output.status);
    let identity = format!("-i {}", identity_key.display());
    let mut tunnels = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if !line.contains(&identity)
            || !line.contains("-o BatchMode=yes")
            || !(line.contains(" -D ") || line.contains(" -L "))
        {
            continue;
        }
        let trimmed = line.trim_start();
        let Some(separator) = trimmed.find(char::is_whitespace) else {
            continue;
        };
        let (pid, command) = trimmed.split_at(separator);
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        let arguments = command.split_whitespace().collect::<Vec<_>>();
        let local_port = arguments.windows(2).find_map(|pair| {
            let endpoint = match pair[0] {
                "-D" => pair[1].strip_prefix("127.0.0.1:")?,
                "-L" => pair[1].strip_prefix("127.0.0.1:")?.split(':').next()?,
                _ => return None,
            };
            endpoint.trim_end_matches(':').parse::<u16>().ok()
        });
        tunnels.push(ManagedSshTunnel { pid, local_port });
    }
    assert!(
        !tunnels.is_empty(),
        "no managed SSH tunnel matched identity key {}",
        identity_key.display()
    );
    for tunnel in &tunnels {
        let status = StdCommand::new("kill")
            .args(["-9", &tunnel.pid.to_string()])
            .status()
            .expect("kill managed SSH tunnel");
        assert!(status.success(), "could not kill SSH pid {}", tunnel.pid);
    }
    tunnels
}

fn reserve_loopback_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind temporary loopback listener")
        .local_addr()
        .expect("temporary listener address")
        .port()
}

fn bracket_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

fn is_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;

    std::fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

pub fn socks5_connect(stream: &mut TcpStream, host: &str, port: u16) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(&[0x05, 0x01, 0x00])?;
    let mut greeting = [0_u8; 2];
    stream.read_exact(&mut greeting)?;
    if greeting != [0x05, 0x00] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("SOCKS5 greeting failed: {greeting:?}"),
        ));
    }

    let mut request = vec![0x05, 0x01, 0x00];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            request.push(0x01);
            request.extend_from_slice(&address.octets());
        }
        Ok(IpAddr::V6(address)) => {
            request.push(0x04);
            request.extend_from_slice(&address.octets());
        }
        Err(_) => {
            let host = host.as_bytes();
            let length = u8::try_from(host.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "SOCKS5 hostname is too long")
            })?;
            request.extend_from_slice(&[0x03, length]);
            request.extend_from_slice(host);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request)?;

    let mut reply = [0_u8; 4];
    stream.read_exact(&mut reply)?;
    if reply[0] != 0x05 || reply[1] != 0x00 {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("SOCKS5 CONNECT failed: {reply:?}"),
        ));
    }
    match reply[3] {
        0x01 => {
            let mut address = [0_u8; 4];
            stream.read_exact(&mut address)?;
        }
        0x04 => {
            let mut address = [0_u8; 16];
            stream.read_exact(&mut address)?;
        }
        0x03 => {
            let mut length = [0_u8; 1];
            stream.read_exact(&mut length)?;
            let mut address = vec![0_u8; usize::from(length[0])];
            stream.read_exact(&mut address)?;
        }
        atyp => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("SOCKS5 returned an unknown address type: {atyp}"),
            ));
        }
    }
    let mut bound_port = [0_u8; 2];
    stream.read_exact(&mut bound_port)?;
    Ok(())
}

pub fn redis_auth_select_ping(stream: &mut (impl Read + Write), password: &str, database: u16) {
    assert_eq!(redis_command(stream, &["AUTH", password]), "+OK");
    assert_eq!(
        redis_command(stream, &["SELECT", &database.to_string()]),
        "+OK"
    );
    assert_eq!(redis_command(stream, &["PING"]), "+PONG");
}

fn redis_command(stream: &mut (impl Read + Write), parts: &[&str]) -> String {
    let mut request = format!("*{}\r\n", parts.len()).into_bytes();
    for part in parts {
        request.extend_from_slice(format!("${}\r\n", part.len()).as_bytes());
        request.extend_from_slice(part.as_bytes());
        request.extend_from_slice(b"\r\n");
    }
    stream.write_all(&request).expect("write Redis request");
    stream.flush().expect("flush Redis request");
    let mut response = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).expect("read Redis response");
        if byte[0] == b'\r' {
            let mut newline = [0u8; 1];
            stream.read_exact(&mut newline).expect("read Redis CRLF");
            assert_eq!(newline[0], b'\n');
            break;
        }
        response.push(byte[0]);
    }
    let response = String::from_utf8(response).expect("Redis response is UTF-8");
    assert!(
        !response.starts_with('-'),
        "Redis returned an error: {response}"
    );
    response
}

pub fn assert_tls_verification_error(error: sqlx::Error, cause: &str, context: &str) {
    let rendered = format!("{error:?}");
    let is_tls_failure = matches!(&error, sqlx::Error::Tls(_))
        || matches!(&error, sqlx::Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidData);
    assert!(
        is_tls_failure,
        "{context}: expected a TLS verification error, got {rendered}"
    );
    assert!(
        rendered.contains(cause),
        "{context}: expected TLS cause {cause}, got {rendered}"
    );
}

fn tail(contents: &str) -> String {
    contents
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}
