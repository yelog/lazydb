use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use tempfile::{NamedTempFile, TempDir};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use tokio_util::sync::CancellationToken;

use crate::{
    connections::DatabaseConnector,
    data_ops::{
        DataJobProgress, DataJobStatus, NativeOutcome, NativeToolVersion, ProgressSink,
        ResolvedEndpoint,
    },
    db::{DatabaseConnection, transport::ConnectionRoute},
};

const CAPTURE_LIMIT: usize = 32 * 1024;

pub(super) fn check_cancel(cancellation: &CancellationToken) -> Result<()> {
    ensure!(!cancellation.is_cancelled(), "data operation cancelled");
    Ok(())
}

pub(super) fn progress(
    sink: &ProgressSink,
    status: DataJobStatus,
    message: impl Into<String>,
    bytes: Option<u64>,
) {
    sink(DataJobProgress {
        status,
        message: message.into(),
        bytes,
        completed_backup: None,
    });
}

pub(super) fn backup_completed(sink: &ProgressSink, outcome: &NativeOutcome) {
    sink(DataJobProgress {
        status: DataJobStatus::Verifying,
        message: "Completed backup artifact published".into(),
        bytes: outcome.bytes,
        completed_backup: Some(outcome.clone()),
    });
}

pub(super) fn clean_text(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && !value.chars().any(char::is_control),
        "names and connection parameters must be nonempty and contain no control characters"
    );
    Ok(())
}

pub(super) fn path_text(path: &Path) -> Result<&str> {
    let value = path
        .to_str()
        .context("native operation paths must be UTF-8")?;
    clean_text(value)?;
    ensure!(
        !value.starts_with("file:") && value != ":memory:",
        "native operations require filesystem paths, not SQLite URIs or memory databases"
    );
    Ok(value)
}

pub(super) fn input_path(path: &Path) -> Result<PathBuf> {
    path_text(path)?;
    let metadata = std::fs::symlink_metadata(path).context("input file is unavailable")?;
    ensure!(
        metadata.file_type().is_file(),
        "input must be a regular file, not a symlink or directory"
    );
    ensure!(metadata.len() > 0, "input file is empty");
    std::fs::canonicalize(path).context("cannot resolve input file")
}

pub(super) fn output_path(path: &Path) -> Result<PathBuf> {
    path_text(path)?;
    let absolute = std::path::absolute(path).context("cannot resolve output path")?;
    let filename = absolute
        .file_name()
        .context("output must name a new file")?;
    let parent = absolute
        .parent()
        .context("output has no parent directory")?;
    let parent = std::fs::canonicalize(parent).context("output parent must already exist")?;
    let metadata = std::fs::metadata(&parent).context("cannot inspect output parent")?;
    ensure!(metadata.is_dir(), "output parent must be a directory");
    ensure!(
        !metadata.permissions().readonly(),
        "output parent is read-only"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o222 != 0,
            "output parent is not writable"
        );
    }
    let result = parent.join(filename);
    absent(&result)?;
    Ok(result)
}

pub(super) fn absent(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => bail!("refusing to overwrite existing file: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("cannot inspect destination path"),
    }
}

pub(super) fn sqlite_output(path: &Path) -> Result<PathBuf> {
    let path = output_path(path)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        absent(Path::new(&sidecar))?;
    }
    Ok(path)
}

pub(super) struct StagedArtifact {
    file: Option<NamedTempFile>,
    path: PathBuf,
}

impl StagedArtifact {
    pub(super) fn new(output: &Path) -> Result<Self> {
        let file = tempfile::Builder::new()
            .prefix(".lazydb-")
            .suffix(".partial")
            .tempfile_in(output.parent().context("output has no parent")?)
            .context("cannot create private partial artifact")?;
        let path = file.path().to_path_buf();
        Ok(Self {
            file: Some(file),
            path,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn writer(&self) -> Result<File> {
        self.file
            .as_ref()
            .context("artifact was already published")?
            .as_file()
            .try_clone()
            .context("cannot open partial artifact")
    }

    pub(super) fn publish(mut self, output: &Path) -> Result<()> {
        absent(output)?;
        self.file
            .as_ref()
            .context("artifact was already published")?
            .as_file()
            .sync_all()
            .context("cannot sync artifact")?;
        let file = self.file.take().context("artifact was already published")?;
        match file.persist_noclobber(output) {
            Ok(file) => {
                file.sync_all().context("cannot sync published artifact")?;
                #[cfg(unix)]
                File::open(output.parent().context("output has no parent")?)?
                    .sync_all()
                    .context("cannot sync artifact directory")?;
                Ok(())
            }
            Err(error) => {
                self.file = Some(error.file);
                Err(error.error).context("exclusive artifact publication failed")
            }
        }
    }
}

impl Drop for StagedArtifact {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.keep();
        }
    }
}

pub(super) async fn copy_file(
    input: &Path,
    output: File,
    cancellation: &CancellationToken,
) -> Result<(u64, String)> {
    check_cancel(cancellation)?;
    let mut input = tokio::fs::File::open(input)
        .await
        .context("cannot open input")?;
    let mut output = tokio::fs::File::from_std(output);
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = tokio::select! {
            biased;
            () = cancellation.cancelled() => bail!("data operation cancelled"),
            result = input.read(&mut buffer) => result.context("cannot read input")?,
        };
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        size += count as u64;
        tokio::select! {
            biased;
            () = cancellation.cancelled() => bail!("data operation cancelled"),
            result = output.write_all(&buffer[..count]) => result.context("cannot write staged file")?,
        }
    }
    output.sync_all().await.context("cannot sync staged file")?;
    check_cancel(cancellation)?;
    Ok((size, format!("{:x}", digest.finalize())))
}

pub(super) async fn fingerprint(
    input: &Path,
    cancellation: &CancellationToken,
) -> Result<(u64, String)> {
    let mut input = tokio::fs::File::open(input)
        .await
        .context("cannot open artifact")?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = tokio::select! {
            biased;
            () = cancellation.cancelled() => bail!("data operation cancelled"),
            result = input.read(&mut buffer) => result.context("cannot hash artifact")?,
        };
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        size += count as u64;
    }
    check_cancel(cancellation)?;
    Ok((size, format!("{:x}", digest.finalize())))
}

pub(super) fn fingerprint_step(bytes: u64, sha256: &str) -> String {
    format!("Input artifact SHA-256: {sha256} ({bytes} bytes)")
}

pub(super) struct PinnedInput {
    file: NamedTempFile,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

impl PinnedInput {
    pub(super) async fn new(path: &Path, cancellation: &CancellationToken) -> Result<Self> {
        let file = tempfile::Builder::new()
            .prefix("lazydb-restore-")
            .suffix(".input")
            .tempfile()
            .context("cannot stage restore input")?;
        let (bytes, sha256) = copy_file(path, file.as_file().try_clone()?, cancellation).await?;
        Ok(Self {
            file,
            bytes,
            sha256,
        })
    }

    pub(super) fn path(&self) -> &Path {
        self.file.path()
    }
}

pub(super) fn reader(path: &Path) -> Result<File> {
    let mut file = File::open(path).context("cannot open native artifact")?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

pub(super) fn signature(path: &Path, expected: &[u8]) -> Result<()> {
    let mut file = reader(path)?;
    let mut actual = vec![0; expected.len()];
    file.read_exact(&mut actual)
        .context("artifact header is incomplete")?;
    ensure!(
        actual == expected,
        "artifact signature does not match the selected engine"
    );
    Ok(())
}

#[derive(Default)]
pub(super) struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub(super) fn new(endpoints: &[Option<&ResolvedEndpoint>]) -> Self {
        let mut secrets = Vec::new();
        for endpoint in endpoints.iter().flatten() {
            if let Some(password) = &endpoint.password {
                let password = password.expose_secret();
                if !password.is_empty() {
                    secrets.extend([
                        password.to_owned(),
                        password.replace('\\', "\\\\").replace(':', "\\:"),
                        option_value(password),
                        percent_encoding::utf8_percent_encode(
                            password,
                            percent_encoding::NON_ALPHANUMERIC,
                        )
                        .to_string(),
                    ]);
                }
            }
        }
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        Self { secrets }
    }

    pub(super) fn redact(&self, value: &[u8]) -> String {
        let value = match std::str::from_utf8(value) {
            Err(error) if error.error_len().is_none() => &value[..error.valid_up_to()],
            _ => value,
        };
        let mut value = String::from_utf8_lossy(value).into_owned();
        for secret in &self.secrets {
            value = value.replace(secret, "[REDACTED]");
            let maximum = secret.len().min(value.len());
            for length in (1..=maximum).rev() {
                if secret.is_char_boundary(length) && value.ends_with(&secret[..length]) {
                    value.truncate(value.len() - length);
                    value.push_str("[REDACTED]");
                    break;
                }
            }
        }
        value
            .chars()
            .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
            .take(CAPTURE_LIMIT)
            .collect()
    }

    pub(super) fn error(&self, error: anyhow::Error) -> anyhow::Error {
        anyhow::anyhow!("{}", self.redact(format!("{error:#}").as_bytes()))
    }
}

pub(super) fn option_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub(super) struct ProcessOutput {
    pub(super) stdout: String,
    pub(super) stderr: String,
}

pub(super) struct Process {
    pub(super) command: Command,
    home: TempDir,
}

impl Process {
    pub(super) fn new(path: &Path) -> Result<Self> {
        let home = tempfile::Builder::new()
            .prefix("lazydb-client-")
            .tempdir()?;
        let mut command = Command::new(path);
        command
            .env_clear()
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .current_dir(home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        Ok(Self { command, home })
    }

    pub(super) fn private_file(&self, name: &str, value: &str) -> Result<PathBuf> {
        let path = self.home.path().join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .context("cannot create client credential file")?;
        file.write_all(value.as_bytes())
            .context("cannot write client credential file")?;
        file.sync_all()?;
        Ok(path)
    }

    pub(super) async fn run(
        &mut self,
        cancellation: &CancellationToken,
        redactor: &Redactor,
        route: Option<&ConnectionRoute>,
        timeout: Option<Duration>,
    ) -> Result<ProcessOutput> {
        check_cancel(cancellation)?;
        if let Some(route) = route {
            route.ensure_alive()?;
        }
        let mut child = self
            .command
            .spawn()
            .context("cannot start native database tool")?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_task = tokio::spawn(capture(stdout));
        let stderr_task = tokio::spawn(capture(stderr));
        let route_failure = async {
            if let Some(lease) = route.and_then(|route| route.lease.as_ref()) {
                lease.failure().await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let deadline = async {
            if let Some(timeout) = timeout {
                tokio::time::sleep(timeout).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let result = tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(anyhow::anyhow!("data operation cancelled")),
            () = route_failure => Err(anyhow::anyhow!("managed SSH route failed")),
            () = deadline => Err(anyhow::anyhow!("native tool version check timed out")),
            status = child.wait() => status.context("cannot wait for native database tool"),
        };
        if result.is_err() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        let stdout = collect_capture(stdout_task).await?;
        let stderr = collect_capture(stderr_task).await?;
        let stdout = redactor.redact(&stdout);
        let stderr = redactor.redact(&stderr);
        let status = result?;
        check_cancel(cancellation)?;
        if let Some(route) = route {
            route.ensure_alive()?;
        }
        ensure!(
            status.success(),
            "native database tool failed ({status}): {stderr}"
        );
        Ok(ProcessOutput { stdout, stderr })
    }
}

async fn capture<R: AsyncRead + Unpin>(reader: Option<R>) -> std::io::Result<Vec<u8>> {
    let Some(mut reader) = reader else {
        return Ok(Vec::new());
    };
    let mut retained = Vec::new();
    let mut buffer = vec![0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let keep = count.min(CAPTURE_LIMIT.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
    }
    Ok(retained)
}

async fn collect_capture(
    mut task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<Vec<u8>> {
    match tokio::time::timeout(Duration::from_secs(2), &mut task).await {
        Ok(result) => Ok(result.context("native diagnostic reader failed")??),
        Err(_) => {
            task.abort();
            let _ = task.await;
            bail!("native diagnostic pipe remained open after process exit")
        }
    }
}

pub(super) async fn tool(
    name: &str,
    configured: Option<&Path>,
    cancellation: &CancellationToken,
    redactor: &Redactor,
) -> Result<NativeToolVersion> {
    let path = executable(configured.unwrap_or_else(|| Path::new(name)))?;
    let mut process = Process::new(&path)?;
    if matches!(name, "mysqldump" | "mysql" | "mariadb-dump" | "mariadb") {
        process.command.arg("--no-defaults");
        if matches!(name, "mysqldump" | "mysql") {
            process.command.arg("--no-login-paths");
        }
    }
    process.command.arg("--version");
    let output = process
        .run(cancellation, redactor, None, Some(Duration::from_secs(10)))
        .await?;
    let version = output.stdout.trim();
    ensure!(!version.is_empty(), "{name} did not report a version");
    Ok(NativeToolVersion {
        name: name.to_owned(),
        path,
        version: version.to_owned(),
    })
}

fn executable(path: &Path) -> Result<PathBuf> {
    let path = if path.components().count() > 1 || path.is_absolute() {
        path.to_path_buf()
    } else {
        let search =
            std::env::var_os("PATH").context("PATH is unavailable; specify native tool paths")?;
        std::env::split_paths(&search)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(path))
            .find(|candidate| is_executable(candidate))
            .with_context(|| {
                format!(
                    "native tool {} is not installed; specify its path",
                    path.display()
                )
            })?
    };
    ensure!(
        is_executable(&path),
        "native tool is not an executable file"
    );
    std::fs::canonicalize(path).context("cannot resolve native executable")
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| {
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            path.extension() == Some(std::ffi::OsStr::new("exe"))
        }
    })
}

pub(super) fn version_number(value: &str) -> Result<(u32, u32, u32)> {
    let value = value
        .split_once("Distrib ")
        .map_or(value, |(_, version)| version);
    let expression = regex::Regex::new(r"(\d+)\.(\d+)(?:\.(\d+))?")?;
    let found = expression
        .captures(value)
        .context("cannot parse database/tool version")?;
    Ok((
        found[1].parse()?,
        found[2].parse()?,
        found.get(3).map_or(Ok(0), |part| part.as_str().parse())?,
    ))
}

pub(super) async fn connect(
    connector: &DatabaseConnector,
    endpoint: &ResolvedEndpoint,
    database: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<DatabaseConnection> {
    check_cancel(cancellation)?;
    let mut profile = endpoint.profile.clone();
    profile.database = database.map(str::to_owned);
    profile.default_schema = None;
    tokio::select! {
        biased;
        () = cancellation.cancelled() => bail!("data operation cancelled"),
        connection = connector.connect(&profile, endpoint.password.as_ref()) => {
            connection.context("cannot connect for native preflight")
        }
    }
}

pub(super) async fn sql(
    connection: &DatabaseConnection,
    statement: &str,
    cancellation: &CancellationToken,
) -> Result<crate::db::query::QueryOutcome> {
    check_cancel(cancellation)?;
    tokio::select! {
        biased;
        () = cancellation.cancelled() => bail!("data operation cancelled"),
        result = connection.execute(statement) => result.context("native database statement failed"),
    }
}

pub(super) fn scalar(result: &crate::db::query::QueryOutcome) -> Result<String> {
    result
        .result_sets
        .first()
        .and_then(|result| result.rows.first())
        .and_then(|row| row.first())
        .map(crate::db::value::CellValue::clipboard_text)
        .context("native database query returned no value")
}

pub(super) fn quote_identifier(value: &str, mysql: bool) -> Result<String> {
    clean_text(value)?;
    if mysql {
        ensure!(
            value.chars().count() <= 64
                && !value.contains(['/', '\\', '.'])
                && !value.ends_with(' '),
            "MySQL/MariaDB database name is invalid or exceeds 64 characters"
        );
        Ok(format!("`{}`", value.replace('`', "``")))
    } else {
        ensure!(
            value.len() <= 63,
            "PostgreSQL database name exceeds 63 bytes"
        );
        Ok(format!("\"{}\"", value.replace('"', "\"\"")))
    }
}

pub(super) fn pg_literal(value: &str) -> String {
    format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

pub(super) fn mysql_literal(value: &str) -> String {
    let hex: String = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("CONVERT(X'{hex}' USING utf8mb4)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_literals_cannot_escape_their_quoting() {
        assert_eq!(
            quote_identifier("a\"; DROP DATABASE x", false).unwrap(),
            "\"a\"\"; DROP DATABASE x\""
        );
        assert_eq!(
            quote_identifier("a`; DROP DATABASE x", true).unwrap(),
            "`a``; DROP DATABASE x`"
        );
        assert!(quote_identifier("a\0b", false).is_err());
        assert!(quote_identifier("x/y", true).is_err());
        assert_eq!(pg_literal("a'\\b"), "E'a''\\\\b'");
        assert_eq!(mysql_literal("a'"), "CONVERT(X'6127' USING utf8mb4)");
        assert_eq!(option_value("a\"\\\nb"), "a\\\"\\\\\\nb");
    }

    #[test]
    fn diagnostics_redact_full_and_truncated_secrets_and_controls() {
        let redactor = Redactor {
            secrets: vec!["sensitive-value".into()],
        };
        assert_eq!(
            redactor.redact(b"\x1b[31msensitive-value sensitive-va"),
            "[31m[REDACTED] [REDACTED]"
        );
        assert!(redactor.redact(&vec![b'x'; 100_000]).len() <= CAPTURE_LIMIT);
        assert_eq!(redactor.redact(b"sensitive-va"), "[REDACTED]");
        let unicode = Redactor {
            secrets: vec!["sensitive😊value".into()],
        };
        assert_eq!(unicode.redact(b"sensitive\xf0\x9f"), "[REDACTED]");
    }

    #[test]
    fn versions_use_the_server_distribution_not_the_mariadb_client_protocol() {
        assert_eq!(
            version_number("mariadb Ver 15.1 Distrib 11.4.9-MariaDB").unwrap(),
            (11, 4, 9)
        );
        assert_eq!(
            version_number("pg_dump (PostgreSQL) 16.11").unwrap(),
            (16, 11, 0)
        );
        assert_eq!(
            version_number("mysqldump Ver 8.4.7 for Linux").unwrap(),
            (8, 4, 7)
        );
    }

    #[test]
    fn publication_never_clobbers_a_racing_file() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("backup");
        let stage = StagedArtifact::new(&target).unwrap();
        stage.writer().unwrap().write_all(b"new").unwrap();
        std::fs::write(&target, b"existing").unwrap();
        assert!(stage.publish(&target).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"existing");
    }

    #[cfg(unix)]
    fn executable_script(directory: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("native-fixture");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_failure_is_bounded_redacted_and_not_successful() {
        let directory = tempfile::tempdir().unwrap();
        let path = executable_script(directory.path(), "printf 'sensitive-value' >&2\nexit 9");
        let mut process = Process::new(&path).unwrap();
        let redactor = Redactor {
            secrets: vec!["sensitive-value".into()],
        };
        let error = process
            .run(&CancellationToken::new(), &redactor, None, None)
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("[REDACTED]"));
        assert!(!error.contains("sensitive-value"));
        assert!(error.contains("failed"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_kills_and_reaps_the_native_process() {
        let directory = tempfile::tempdir().unwrap();
        let pid_path = directory.path().join("pid");
        let path = executable_script(
            directory.path(),
            &format!(
                "printf '%s' $$ > '{}'\nexec /bin/sleep 30",
                pid_path.display()
            ),
        );
        let token = CancellationToken::new();
        let child_token = token.clone();
        let task = tokio::spawn(async move {
            Process::new(&path)
                .unwrap()
                .run(&child_token, &Redactor::default(), None, None)
                .await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while !pid_path.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "native fixture never started"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let pid = std::fs::read_to_string(&pid_path).unwrap();
        token.cancel();
        let error = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .err()
            .unwrap();
        assert!(error.to_string().contains("cancelled"));
        assert!(
            !std::process::Command::new("/bin/kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
}
