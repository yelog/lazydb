use std::{
    collections::VecDeque,
    ffi::OsString,
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::watch,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::{
    config::SshConfig,
    db::transport::TransportEndpoint,
    profile::{ConnectionProfile, DatabaseKind, SshOptions, SslMode},
};

use super::{
    error::{SshError, SshErrorKind, classify_output},
    manager::TunnelStatus,
};

const MAX_CONFIG_BYTES: u64 = 256 * 1024;
const MAX_LOG_BYTES: usize = 64 * 1024;
const MAX_LINE_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ForwardMode {
    Tcp,
    PgUnix,
    MySqlUnix,
    Socks,
}

#[derive(Clone, Debug)]
pub(crate) struct LaunchOptions {
    pub(crate) ssh: SshOptions,
    pub(crate) mode: ForwardMode,
    pub(crate) destination: Option<(String, u16)>,
    pub(crate) config: SshConfig,
}

impl LaunchOptions {
    pub(crate) fn from_profile(
        profile: &ConnectionProfile,
        config: SshConfig,
    ) -> Result<Self, SshError> {
        profile.validate_ssh().map_err(|_| {
            SshError::new(
                SshErrorKind::Configuration,
                "SSH profile configuration is invalid or unsupported for this database",
            )
        })?;
        let ssh = profile.ssh.clone().ok_or_else(|| {
            SshError::new(SshErrorKind::Configuration, "SSH configuration is missing")
        })?;
        let mode = match profile.kind {
            DatabaseKind::Postgres | DatabaseKind::CockroachDb => {
                #[cfg(unix)]
                {
                    sqlx_forward_mode(profile.ssl_mode, ForwardMode::PgUnix)
                }
                #[cfg(not(unix))]
                {
                    reject_sqlx_identity_verification(profile)?;
                    ForwardMode::Tcp
                }
            }
            DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb => {
                #[cfg(unix)]
                {
                    sqlx_forward_mode(profile.ssl_mode, ForwardMode::MySqlUnix)
                }
                #[cfg(not(unix))]
                {
                    reject_sqlx_identity_verification(profile)?;
                    ForwardMode::Tcp
                }
            }
            DatabaseKind::SqlServer | DatabaseKind::Redis | DatabaseKind::Valkey => {
                ForwardMode::Tcp
            }
            DatabaseKind::ClickHouse
            | DatabaseKind::Elasticsearch
            | DatabaseKind::OpenSearch
            | DatabaseKind::MongoDb => ForwardMode::Socks,
            _ => {
                return Err(SshError::new(
                    SshErrorKind::Unsupported,
                    "SSH tunnels are not supported for this database driver",
                ));
            }
        };
        let destination = if mode == ForwardMode::Socks {
            None
        } else {
            let host = profile
                .host
                .as_deref()
                .filter(|host| valid_destination(host))
                .ok_or_else(|| {
                    SshError::new(
                        SshErrorKind::Configuration,
                        "SSH forwarding requires a valid database hostname or IP address",
                    )
                })?;
            let default_port = match profile.kind {
                DatabaseKind::Postgres => 5432,
                DatabaseKind::CockroachDb => 26257,
                DatabaseKind::MySql | DatabaseKind::MariaDb => 3306,
                DatabaseKind::TiDb => 4000,
                DatabaseKind::SqlServer => 1433,
                DatabaseKind::Redis | DatabaseKind::Valkey => 6379,
                _ => 0,
            };
            let port = profile.port.unwrap_or(default_port);
            if port == 0 {
                return Err(SshError::new(
                    SshErrorKind::Configuration,
                    "SSH forwarding requires a non-zero database port",
                ));
            }
            Some((host.to_owned(), port))
        };
        Ok(Self {
            ssh,
            mode,
            destination,
            config,
        })
    }
}

#[cfg(not(unix))]
fn reject_sqlx_identity_verification(profile: &ConnectionProfile) -> Result<(), SshError> {
    if matches!(profile.ssl_mode, SslMode::VerifyCa | SslMode::VerifyFull) {
        return Err(SshError::new(
            SshErrorKind::Unsupported,
            "This platform's SQLx TCP transport cannot preserve the database TLS hostname through SSH; verify-ca and verify-full tunneling require the Unix socket transport",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn sqlx_forward_mode(ssl_mode: SslMode, unix_mode: ForwardMode) -> ForwardMode {
    if matches!(ssl_mode, SslMode::Disable) {
        ForwardMode::Tcp
    } else {
        unix_mode
    }
}

fn valid_destination(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_:%[]".contains(&byte))
}

pub(crate) fn find_program() -> Result<PathBuf, SshError> {
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let candidate = PathBuf::from(root).join("System32/OpenSSH/ssh.exe");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let name = if cfg!(windows) { "ssh.exe" } else { "ssh" };
    for directory in std::env::var_os("PATH")
        .as_deref()
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
    {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(name);
        if candidate.is_file() {
            return candidate.canonicalize().map_err(|_| unavailable());
        }
    }
    Err(unavailable())
}

fn unavailable() -> SshError {
    SshError::new(
        SshErrorKind::Unavailable,
        "OpenSSH client was not found; install an OpenSSH client and make ssh available on PATH",
    )
}

fn expanded_path(path: &Path) -> Result<PathBuf, SshError> {
    let expanded = if let Ok(suffix) = path.strip_prefix("~") {
        directories::BaseDirs::new()
            .ok_or_else(|| {
                SshError::new(
                    SshErrorKind::Configuration,
                    "The SSH path's home directory could not be resolved",
                )
            })?
            .home_dir()
            .join(suffix)
    } else {
        std::path::absolute(path).map_err(|_| {
            SshError::new(
                SshErrorKind::Configuration,
                "The SSH file path could not be resolved",
            )
        })?
    };
    if !expanded.is_file() {
        return Err(SshError::new(
            SshErrorKind::Configuration,
            "An explicitly configured SSH key, config or known_hosts file is unavailable",
        ));
    }
    Ok(expanded)
}

fn quoted_config_path(path: &Path) -> Result<String, SshError> {
    let path = path.to_str().filter(|path| {
        !path
            .chars()
            .any(|character| character.is_control() || character == '%')
    });
    let path = path.ok_or_else(|| {
        SshError::new(
            SshErrorKind::Configuration,
            "SSH known_hosts paths must be Unicode and cannot contain control characters or percent expansion",
        )
    })?;
    Ok(format!(
        "\"{}\"",
        path.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

pub(crate) fn command(
    program: &Path,
    options: &LaunchOptions,
    preflight: bool,
) -> Result<Command, SshError> {
    let mut command = Command::new(program);
    command
        .stdin(Stdio::null())
        .stdout(if preflight {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        .env("LC_ALL", "C")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env_remove("SSH_ASKPASS");
    command.args(["-N", "-T"]);
    if preflight {
        command.arg("-G");
    }
    for option in [
        "BatchMode=yes",
        "NumberOfPasswordPrompts=0",
        "ForwardAgent=no",
        "ForwardX11=no",
        "RequestTTY=no",
        "RemoteCommand=none",
        "PermitLocalCommand=no",
        "LocalCommand=none",
        "ControlMaster=no",
        "ControlPersist=no",
        "ControlPath=none",
        "ForkAfterAuthentication=no",
        "ExitOnForwardFailure=yes",
        "StrictHostKeyChecking=yes",
        "ClearAllForwardings=no",
        "ConnectionAttempts=1",
        "GatewayPorts=no",
        "LogLevel=DEBUG1",
    ] {
        command.arg("-o").arg(option);
    }
    for option in [
        format!("ConnectTimeout={}", options.config.connect_timeout_seconds),
        format!(
            "ServerAliveInterval={}",
            options.config.server_alive_interval_seconds
        ),
        format!(
            "ServerAliveCountMax={}",
            options.config.server_alive_count_max
        ),
    ] {
        command.arg("-o").arg(option);
    }
    if let Some(path) = &options.ssh.config_file {
        command.arg("-F").arg(expanded_path(path)?);
    }
    if let Some(path) = &options.ssh.identity_file {
        command
            .arg("-i")
            .arg(expanded_path(path)?)
            .args(["-o", "IdentitiesOnly=yes"]);
    }
    if let Some(path) = &options.ssh.known_hosts_file {
        command.arg("-o").arg(format!(
            "UserKnownHostsFile={}",
            quoted_config_path(&expanded_path(path)?)?
        ));
    }
    if let Some(port) = options.ssh.port {
        command.arg("-p").arg(port.to_string());
    }
    if let Some(user) = &options.ssh.user {
        command.arg("-l").arg(user);
    }
    if let Some(jump) = &options.ssh.proxy_jump {
        command.arg("-J").arg(jump);
    }
    Ok(command)
}

pub(crate) struct OwnedChild {
    child: Box<dyn ChildWrapper>,
    reaped: bool,
    kill_sent: bool,
}

impl OwnedChild {
    pub(crate) fn spawn(command: Command) -> Result<Self, SshError> {
        let mut wrapped = CommandWrap::from(command);
        #[cfg(unix)]
        wrapped.wrap(process_wrap::tokio::ProcessSession);
        #[cfg(windows)]
        {
            let mut flags = process_wrap::tokio::CreationFlags(Default::default());
            flags.0.0 = 0x08000000;
            wrapped.wrap(flags).wrap(process_wrap::tokio::JobObject);
        }
        wrapped.wrap(KillOnDrop);
        let child = wrapped.spawn().map_err(|_| unavailable())?;
        Ok(Self {
            child,
            reaped: false,
            kill_sent: false,
        })
    }

    async fn terminate(&mut self) {
        self.kill_group();
        if self.reaped {
            return;
        }
        if tokio::time::timeout(Duration::from_secs(2), self.child.wait())
            .await
            .is_ok_and(|result| result.is_ok())
        {
            self.reaped = true;
        }
    }

    fn kill_group(&mut self) {
        if !self.kill_sent {
            let _ = self.child.start_kill();
            self.kill_sent = true;
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.kill_group();
    }
}

async fn bounded_output(reader: Option<impl AsyncRead + Unpin>) -> Result<Vec<u8>, std::io::Error> {
    let mut output = Vec::new();
    if let Some(reader) = reader {
        reader
            .take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut output)
            .await?;
    }
    Ok(output)
}

pub(crate) async fn preflight(
    program: &Path,
    options: &LaunchOptions,
    stop: &CancellationToken,
    deadline: Instant,
) -> Result<(), SshError> {
    let mut command = command(program, options, true)?;
    command.arg(&options.ssh.host);
    let mut child = OwnedChild::spawn(command)?;
    let stdout = child.child.stdout().take();
    let stderr = child.child.stderr().take();
    let result = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(SshError::cancelled()),
        _ = tokio::time::sleep_until(deadline) => Err(SshError::timeout()),
        output = async {
            tokio::join!(child.child.wait(), bounded_output(stdout), bounded_output(stderr))
        } => {
            let (status, stdout, stderr) = output;
            match (status, stdout, stderr) {
                (Ok(status), Ok(stdout), Ok(stderr)) => {
                    child.reaped = true;
                    if stdout.len() as u64 > MAX_CONFIG_BYTES || stderr.len() as u64 > MAX_CONFIG_BYTES {
                        Err(SshError::new(SshErrorKind::Configuration, "SSH configuration output exceeded its size limit"))
                    } else if !status.success() {
                        Err(classify_output(&String::from_utf8_lossy(&stderr)).unwrap_or_else(|| {
                            SshError::new(SshErrorKind::Configuration, "SSH configuration could not be evaluated")
                        }))
                    } else {
                        validate_effective_config(&String::from_utf8_lossy(&stdout))
                    }
                }
                _ => Err(SshError::new(SshErrorKind::Configuration, "SSH configuration could not be evaluated")),
            }
        }
    };
    child.terminate().await;
    result
}

fn validate_effective_config(output: &str) -> Result<(), SshError> {
    if !output.lines().any(|line| line.starts_with("hostname ")) {
        return Err(SshError::new(
            SshErrorKind::Unsupported,
            "The SSH client did not return a compatible OpenSSH configuration",
        ));
    }
    if output.lines().any(|line| {
        matches!(
            line.split_ascii_whitespace().next(),
            Some("localforward" | "remoteforward" | "dynamicforward")
        )
    }) {
        return Err(SshError::new(
            SshErrorKind::Configuration,
            "The SSH alias inherits additional forwards; use a dedicated alias without LocalForward, RemoteForward or DynamicForward",
        ));
    }
    Ok(())
}

struct ForwardSpec {
    endpoint: TransportEndpoint,
    arguments: Vec<OsString>,
    marker: String,
    listener_type: &'static str,
    _directory: Option<tempfile::TempDir>,
}

impl ForwardSpec {
    fn new(options: &LaunchOptions) -> Result<Self, SshError> {
        if matches!(options.mode, ForwardMode::PgUnix | ForwardMode::MySqlUnix) {
            let (host, port) = options.destination.as_ref().ok_or_else(|| {
                SshError::new(
                    SshErrorKind::Configuration,
                    "SSH database destination is missing",
                )
            })?;
            let directory = tempfile::Builder::new()
                .prefix("ldssh-")
                .tempdir()
                .map_err(|_| {
                    SshError::new(
                        SshErrorKind::Configuration,
                        "A private SSH socket directory could not be created",
                    )
                })?;
            let filename = if options.mode == ForwardMode::PgUnix {
                format!(".s.PGSQL.{port}")
            } else {
                "mysql.sock".to_owned()
            };
            let path = directory.path().join(filename);
            if path.as_os_str().len() >= 100 {
                return Err(SshError::new(
                    SshErrorKind::Configuration,
                    "The SSH Unix socket path is too long; configure a shorter temporary directory",
                ));
            }
            let marker = path.to_string_lossy().into_owned();
            let forward = format!("{}:{}:{port}", marker, bracket_host(host));
            return Ok(Self {
                endpoint: TransportEndpoint::Unix(path),
                arguments: vec![
                    "-o".into(),
                    "StreamLocalBindMask=0177".into(),
                    "-o".into(),
                    "StreamLocalBindUnlink=no".into(),
                    "-L".into(),
                    forward.into(),
                ],
                marker,
                listener_type: "new unix-listener",
                _directory: Some(directory),
            });
        }
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| {
            SshError::new(
                SshErrorKind::Bind,
                "A private loopback port could not be allocated for SSH",
            )
        })?;
        let endpoint = reservation.local_addr().map_err(|_| {
            SshError::new(
                SshErrorKind::Bind,
                "The SSH loopback port could not be read",
            )
        })?;
        let (flag, target) = if options.mode == ForwardMode::Socks {
            ("-D", endpoint.to_string())
        } else {
            let (host, port) = options.destination.as_ref().ok_or_else(|| {
                SshError::new(
                    SshErrorKind::Configuration,
                    "SSH database destination is missing",
                )
            })?;
            ("-L", format!("{endpoint}:{}:{port}", bracket_host(host)))
        };
        drop(reservation);
        Ok(Self {
            endpoint: if options.mode == ForwardMode::Socks {
                TransportEndpoint::Socks5(endpoint)
            } else {
                TransportEndpoint::Tcp(endpoint)
            },
            arguments: vec![flag.into(), target.into()],
            marker: format!("127.0.0.1 port {}", endpoint.port()),
            listener_type: "new port-listener",
            _directory: None,
        })
    }
}

fn bracket_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

#[derive(Default)]
struct DiagnosticBuffer {
    bytes: VecDeque<u8>,
    line: Vec<u8>,
    long_line: bool,
}

impl DiagnosticBuffer {
    fn push(&mut self, bytes: &[u8], mut on_line: impl FnMut(&str)) {
        for &byte in bytes {
            if self.bytes.len() == MAX_LOG_BYTES {
                self.bytes.pop_front();
            }
            self.bytes.push_back(byte);
            if byte == b'\n' {
                if !self.long_line {
                    on_line(&String::from_utf8_lossy(&self.line));
                }
                self.line.clear();
                self.long_line = false;
            } else if self.line.len() < MAX_LINE_BYTES {
                self.line.push(byte);
            } else {
                self.long_line = true;
            }
        }
    }

    fn push_tail(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes.iter().copied());
        let excess = self.bytes.len().saturating_sub(MAX_LOG_BYTES);
        self.bytes.drain(..excess);
    }

    fn error(&self) -> Option<SshError> {
        classify_output(&String::from_utf8_lossy(
            &self.bytes.iter().copied().collect::<Vec<_>>(),
        ))
    }
}

struct ReadyEvidence {
    marker: String,
    listener_type: &'static str,
    forward_seen: bool,
    listener_seen: bool,
}

impl ReadyEvidence {
    fn observe(&mut self, line: &str) -> bool {
        if line.starts_with("debug1: Local forwarding listening on ") && line.contains(&self.marker)
        {
            self.forward_seen = true;
        }
        if self.forward_seen
            && line.starts_with("debug1: channel ")
            && line.contains(self.listener_type)
        {
            self.listener_seen = true;
        }
        self.listener_seen && line == "debug1: Entering interactive session."
    }
}

pub(crate) async fn supervise(
    program: PathBuf,
    options: LaunchOptions,
    state: watch::Sender<TunnelStatus>,
    stop: CancellationToken,
) -> Result<(), SshError> {
    let deadline = Instant::now() + Duration::from_secs(options.config.connect_timeout_seconds);
    preflight(&program, &options, &stop, deadline).await?;
    for attempt in 0..3 {
        let spec = ForwardSpec::new(&options)?;
        let result = run_forward(&program, &options, &spec, &state, &stop, deadline).await;
        if result
            .as_ref()
            .is_err_and(|error| error.kind == SshErrorKind::Bind)
            && matches!(*state.borrow(), TunnelStatus::Starting)
            && attempt < 2
            && !stop.is_cancelled()
            && Instant::now() < deadline
        {
            continue;
        }
        return result;
    }
    Err(SshError::new(
        SshErrorKind::Bind,
        "SSH forwarding could not be started",
    ))
}

async fn run_forward(
    program: &Path,
    options: &LaunchOptions,
    spec: &ForwardSpec,
    state: &watch::Sender<TunnelStatus>,
    stop: &CancellationToken,
    deadline: Instant,
) -> Result<(), SshError> {
    let mut command = command(program, options, false)?;
    command.args(&spec.arguments).arg(&options.ssh.host);
    let mut child = OwnedChild::spawn(command)?;
    let mut stderr = child.child.stderr().take().ok_or_else(|| {
        SshError::new(
            SshErrorKind::Unsupported,
            "SSH startup diagnostics are unavailable",
        )
    })?;
    let mut diagnostics = DiagnosticBuffer::default();
    let mut evidence = ReadyEvidence {
        marker: spec.marker.clone(),
        listener_type: spec.listener_type,
        forward_seen: false,
        listener_seen: false,
    };
    let mut buffer = [0; 64 * 1024];
    let mut ready = false;
    let mut eof = false;
    let result = loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break Ok(()),
            _ = tokio::time::sleep_until(deadline), if !ready => break Err(SshError::timeout()),
            result = stderr.read(&mut buffer), if !eof => {
                match result {
                    Ok(0) => eof = true,
                    Ok(count) => {
                        if ready {
                            diagnostics.push_tail(&buffer[..count]);
                        } else {
                            let mut observed_ready = false;
                            diagnostics.push(&buffer[..count], |line| {
                                observed_ready |= evidence.observe(line.trim_end_matches('\r'));
                            });
                            if observed_ready {
                                if child.child.try_wait().ok().flatten().is_some() {
                                    child.reaped = true;
                                    break Err(diagnostics.error().unwrap_or_else(SshError::exited));
                                }
                                ready = true;
                                state.send_replace(TunnelStatus::Ready(spec.endpoint.clone()));
                            }
                        }
                    }
                    Err(_) => break Err(SshError::new(
                        SshErrorKind::Network,
                        "SSH startup diagnostics could not be read",
                    )),
                }
            }
            result = child.child.wait() => {
                child.reaped = result.is_ok();
                break Err(diagnostics.error().unwrap_or_else(SshError::exited));
            }
        }
    };
    if ready {
        state.send_replace(match &result {
            Ok(()) => TunnelStatus::Stopped,
            Err(error) => TunnelStatus::Failed(error.clone()),
        });
    }
    child.terminate().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    async fn process_group_is_running(group: u32) -> bool {
        let output = Command::new("ps")
            .args(["-axo", "pgid=,stat="])
            .output()
            .await
            .expect("inspect the disposable process group");
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut columns = line.split_whitespace();
            columns.next().and_then(|value| value.parse::<u32>().ok()) == Some(group)
                && columns.next().is_some_and(|state| !state.starts_with('Z'))
        })
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reaping_the_leader_does_not_leave_its_process_group_running() {
        for explicitly_terminate in [true, false] {
            let mut command = Command::new("sh");
            command
                .args(["-c", "sleep 60 >/dev/null 2>&1 &"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let mut child = OwnedChild::spawn(command).unwrap();
            let group = child.child.id().expect("disposable process group ID");
            tokio::time::timeout(Duration::from_secs(2), child.child.wait())
                .await
                .expect("process leader did not exit")
                .expect("wait for the process leader");
            child.reaped = true;
            assert!(process_group_is_running(group).await);
            if explicitly_terminate {
                child.terminate().await;
            }
            drop(child);
            let deadline = Instant::now() + Duration::from_secs(2);
            while process_group_is_running(group).await {
                assert!(
                    Instant::now() < deadline,
                    "an owned SSH descendant survived cleanup"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }

    #[test]
    fn listening_message_alone_cannot_mark_a_forward_ready() {
        let mut evidence = ReadyEvidence {
            marker: "127.0.0.1 port 12345".to_owned(),
            listener_type: "new port-listener",
            forward_seen: false,
            listener_seen: false,
        };
        assert!(!evidence.observe("debug1: Entering interactive session."));
        assert!(!evidence.observe("debug1: Local forwarding listening on 127.0.0.1 port 12345."));
        assert!(!evidence.observe("bind [127.0.0.1]:12345: Address already in use"));
        assert!(!evidence.observe("debug1: Entering interactive session."));
        assert!(!evidence.observe("debug1: channel 0: new port-listener [port listener]"));
        assert!(evidence.observe("debug1: Entering interactive session."));
    }

    #[test]
    fn another_forward_or_jump_cannot_mark_the_target_ready() {
        let mut evidence = ReadyEvidence {
            marker: "127.0.0.1 port 12345".to_owned(),
            listener_type: "new port-listener",
            forward_seen: false,
            listener_seen: false,
        };
        for line in [
            "debug1: Local forwarding listening on 127.0.0.1 port 12346.",
            "debug1: channel 0: new port-listener [port listener]",
            "debug1: Entering interactive session.",
            "debug1: channel 0: new stdio-forward [stdio-forward]",
        ] {
            assert!(!evidence.observe(line));
        }
    }

    #[test]
    fn inherited_forwards_are_rejected_without_clearing_managed_forwards() {
        assert!(
            validate_effective_config("hostname bastion\nlocalforward 12345 db:5432\n").is_err()
        );
        assert!(
            validate_effective_config("hostname bastion\nremoteforward 12345 db:5432\n").is_err()
        );
        assert!(validate_effective_config("hostname bastion\ndynamicforward 12345\n").is_err());
        assert!(validate_effective_config("hostname bastion\nclearallforwardings no\n").is_ok());
    }

    #[test]
    fn diagnostic_storage_and_unterminated_lines_are_bounded() {
        let mut buffer = DiagnosticBuffer::default();
        let mut complete = 0;
        buffer.push(&vec![b'x'; MAX_LOG_BYTES * 2], |_| complete += 1);
        assert_eq!(buffer.bytes.len(), MAX_LOG_BYTES);
        assert_eq!(buffer.line.len(), MAX_LINE_BYTES);
        assert_eq!(complete, 0);
        buffer.push(b"\nPermission denied (publickey).\n", |_| complete += 1);
        assert_eq!(complete, 1);
        assert_eq!(buffer.error().unwrap().kind, SshErrorKind::Authentication);
    }

    #[test]
    fn database_forwarding_target_cannot_inject_another_argument() {
        for host in ["db\n-L 0.0.0.0:22:x:22", "-oProxyCommand=bad", "db;bad", ""] {
            assert!(!valid_destination(host));
        }
        for host in ["db.internal", "127.0.0.1", "2001:db8::1", "[::1]"] {
            assert!(valid_destination(host));
        }
        assert_eq!(bracket_host("2001:db8::1"), "[2001:db8::1]");
    }

    #[cfg(unix)]
    #[test]
    fn sqlx_tls_modes_use_unix_forwarding_to_preserve_the_logical_hostname() {
        assert_eq!(
            sqlx_forward_mode(SslMode::Disable, ForwardMode::PgUnix),
            ForwardMode::Tcp
        );
        assert_eq!(
            sqlx_forward_mode(SslMode::Disable, ForwardMode::MySqlUnix),
            ForwardMode::Tcp
        );
        for ssl_mode in [
            SslMode::Prefer,
            SslMode::Require,
            SslMode::VerifyCa,
            SslMode::VerifyFull,
        ] {
            assert_eq!(
                sqlx_forward_mode(ssl_mode, ForwardMode::PgUnix),
                ForwardMode::PgUnix
            );
            assert_eq!(
                sqlx_forward_mode(ssl_mode, ForwardMode::MySqlUnix),
                ForwardMode::MySqlUnix
            );
        }
    }

    #[test]
    fn tidb_default_forward_port_matches_its_adapter() {
        let mut imported =
            crate::profile::import_connection_url("mysql://root@tidb.internal/app", None).unwrap();
        imported.profile.kind = DatabaseKind::TiDb;
        imported.profile.port = None;
        imported.profile.ssh = Some(SshOptions {
            host: "bastion.example.test".into(),
            ..SshOptions::default()
        });

        let options = LaunchOptions::from_profile(&imported.profile, SshConfig::default()).unwrap();
        assert_eq!(
            options.destination,
            Some(("tidb.internal".to_owned(), 4000))
        );
    }
}
