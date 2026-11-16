use std::{
    collections::VecDeque,
    fs::File,
    io::{BufReader, Bytes, Read, Seek, SeekFrom},
    iter::Peekable,
    path::Path,
    process::Stdio,
};

use anyhow::{Context, Result, bail, ensure};
use secrecy::ExposeSecret;
use tokio_util::sync::CancellationToken;

use crate::{
    connections::DatabaseConnector,
    data_ops::{
        DataJobSpec, DataJobStatus, DataOperation, NativeOutcome, NativePlan, NativeToolVersion,
        ProgressSink, ResolvedEndpoint,
    },
    db::{
        DatabaseConnection,
        transport::{ConnectionRoute, TransportEndpoint},
    },
    profile::{DatabaseKind, SslMode},
};

use super::common::{self, PinnedInput, Process, Redactor, StagedArtifact};

fn names(engine: DatabaseKind) -> (&'static str, &'static str) {
    if engine == DatabaseKind::MariaDb {
        ("mariadb-dump", "mariadb")
    } else {
        ("mysqldump", "mysql")
    }
}

fn tls_arguments(endpoint: &ResolvedEndpoint) -> Result<Vec<&'static str>> {
    let maria = endpoint.profile.kind == DatabaseKind::MariaDb;
    match (maria, endpoint.profile.ssl_mode) {
        (false, SslMode::Disable) => Ok(vec!["--ssl-mode=DISABLED"]),
        (false, SslMode::Prefer) => Ok(vec!["--ssl-mode=PREFERRED"]),
        (false, SslMode::Require) => Ok(vec!["--ssl-mode=REQUIRED"]),
        (true, SslMode::Disable) => Ok(vec!["--skip-ssl"]),
        (true, SslMode::Prefer) => Ok(vec!["--ssl", "--skip-ssl-verify-server-cert"]),
        (true, SslMode::Require) => bail!(
            "MariaDB native clients cannot guarantee this profile's require-TLS policy without an explicit CA/client TLS configuration; use a supported TLS profile"
        ),
        (_, SslMode::VerifyCa | SslMode::VerifyFull) => bail!(
            "native MySQL/MariaDB certificate verification cannot reproduce SQLx system-root trust from this profile; explicit native CA configuration is not supported"
        ),
    }
}

fn client(
    tool: &NativeToolVersion,
    endpoint: &ResolvedEndpoint,
    route: &ConnectionRoute,
) -> Result<Process> {
    let mut process = Process::new(&tool.path)?;
    let password = endpoint
        .password
        .as_ref()
        .map(ExposeSecret::expose_secret)
        .unwrap_or("");
    ensure!(
        !password.contains('\0'),
        "MySQL/MariaDB option files cannot represent passwords containing NUL"
    );
    let defaults = process.private_file(
        "client.cnf",
        &format!(
            "[client]\npassword=\"{}\"\n",
            common::option_value(password)
        ),
    )?;
    let (host, port) = match &route.endpoint {
        TransportEndpoint::Direct => (
            endpoint
                .profile
                .host
                .clone()
                .context("MySQL/MariaDB host is missing")?,
            endpoint.profile.port.unwrap_or(3306),
        ),
        TransportEndpoint::Tcp(address) => (address.ip().to_string(), address.port()),
        TransportEndpoint::Unix(_) | TransportEndpoint::Socks5(_) => {
            bail!("native MySQL/MariaDB requires a direct or managed TCP route to preserve TLS")
        }
    };
    process
        .command
        .arg(format!("--defaults-file={}", common::path_text(&defaults)?));
    if endpoint.profile.kind == DatabaseKind::MySql {
        process.command.arg("--no-login-paths");
    }
    process
        .command
        .arg("--protocol=TCP")
        .arg(format!("--host={host}"))
        .arg(format!("--port={port}"))
        .arg(format!(
            "--user={}",
            endpoint
                .profile
                .user
                .as_deref()
                .context("MySQL/MariaDB user is missing")?
        ))
        .arg("--default-character-set=utf8mb4")
        .args(tls_arguments(endpoint)?);
    if matches!(tool.name.as_str(), "mysql" | "mariadb") {
        process.command.arg("--connect-timeout=10");
    }
    Ok(process)
}

fn dump_arguments(engine: DatabaseKind, database: &str) -> Vec<String> {
    let mut arguments: Vec<String> = [
        "--single-transaction",
        "--quick",
        "--routines",
        "--events",
        "--triggers",
        "--hex-blob",
        "--skip-lock-tables",
        "--skip-add-locks",
        "--skip-add-drop-table",
        "--no-create-db",
        "--no-tablespaces",
        "--comments",
        "--dump-date",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if engine == DatabaseKind::MySql {
        arguments.push("--set-gtid-purged=OFF".into());
    }
    arguments.push("--".into());
    arguments.push(database.into());
    arguments
}

fn restore_arguments(database: &str) -> Vec<String> {
    vec![
        "--binary-mode".into(),
        "--batch".into(),
        "--skip-reconnect".into(),
        "--local-infile=0".into(),
        format!("--database={database}"),
    ]
}

async fn route(
    connector: &DatabaseConnector,
    endpoint: &ResolvedEndpoint,
    cancellation: &CancellationToken,
) -> Result<ConnectionRoute> {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => bail!("data operation cancelled"),
        route = connector.native_tcp_route(&endpoint.profile) => route.context("cannot acquire native MySQL/MariaDB route"),
    }
}

fn check_family(version: &str, engine: DatabaseKind) -> Result<(u32, u32, u32)> {
    let maria = version.to_ascii_lowercase().contains("mariadb");
    ensure!(
        maria == (engine == DatabaseKind::MariaDb),
        "native client/server belongs to a different database engine"
    );
    ensure!(
        !version.to_ascii_lowercase().contains("tidb"),
        "TiDB is not a native MySQL data-operation target"
    );
    let version = version.strip_prefix("5.5.5-").unwrap_or(version);
    common::version_number(version)
}

async fn server_version(
    connection: &DatabaseConnection,
    engine: DatabaseKind,
    cancellation: &CancellationToken,
) -> Result<(u32, u32, u32)> {
    let version =
        common::scalar(&common::sql(connection, "SELECT VERSION()", cancellation).await?)?;
    check_family(&version, engine)
}

async fn source_version(
    endpoint: &ResolvedEndpoint,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<(u32, u32, u32)> {
    let connection =
        common::connect(connector, endpoint, Some(&endpoint.database), cancellation).await?;
    let result = async {
        let database =
            common::scalar(&common::sql(&connection, "SELECT DATABASE()", cancellation).await?)?;
        ensure!(
            database == endpoint.database,
            "source connection selected a different database"
        );
        server_version(&connection, endpoint.profile.kind, cancellation).await
    }
    .await;
    connection.close().await;
    result
}

async fn ensure_destination_absent(
    connection: &DatabaseConnection,
    database: &str,
    cancellation: &CancellationToken,
) -> Result<()> {
    let statement = format!(
        "SELECT count(*) FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = {}",
        common::mysql_literal(database)
    );
    let count = common::scalar(&common::sql(connection, &statement, cancellation).await?)?;
    ensure!(
        count == "0",
        "MySQL/MariaDB destination database already exists"
    );
    Ok(())
}

async fn destination_version(
    endpoint: &ResolvedEndpoint,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<(u32, u32, u32)> {
    let connection = common::connect(connector, endpoint, None, cancellation).await?;
    let result = async {
        ensure_destination_absent(&connection, &endpoint.database, cancellation).await?;
        server_version(&connection, endpoint.profile.kind, cancellation).await
    }
    .await;
    connection.close().await;
    result
}

struct DumpMetadata {
    version: (u32, u32, u32),
    database: String,
}

fn dump_metadata(path: &Path, engine: DatabaseKind) -> Result<DumpMetadata> {
    let mut file = File::open(path)?;
    let mut header = vec![0_u8; 32 * 1024];
    let count = file.read(&mut header)?;
    let header = String::from_utf8_lossy(&header[..count]);
    let identity = header
        .lines()
        .find(|line| line.starts_with("-- MySQL dump ") || line.starts_with("-- MariaDB dump "))
        .context("SQL artifact is missing its native MySQL/MariaDB dump signature")?;
    check_family(identity, engine)?;
    let server = header
        .lines()
        .find_map(|line| line.strip_prefix("-- Server version"))
        .context("SQL artifact is missing its source server version")?;
    let version = check_family(server.trim(), engine)?;
    let database = header
        .lines()
        .find(|line| line.starts_with("-- Host:"))
        .and_then(|line| line.split_once("Database:"))
        .map(|(_, database)| database.trim().to_owned())
        .context("SQL artifact is missing the source database name")?;
    common::quote_identifier(&database, true)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(32 * 1024)))?;
    let mut tail = Vec::new();
    file.take(32 * 1024).read_to_end(&mut tail)?;
    ensure!(
        String::from_utf8_lossy(&tail)
            .lines()
            .any(|line| line.starts_with("-- Dump completed")),
        "SQL artifact is incomplete: native dump completion marker is missing"
    );
    Ok(DumpMetadata { version, database })
}

struct SqlBytes {
    bytes: Peekable<Bytes<BufReader<File>>>,
    cancellation: CancellationToken,
    count: usize,
}

impl SqlBytes {
    fn new(path: &Path, cancellation: &CancellationToken) -> Result<Self> {
        Ok(Self {
            bytes: BufReader::with_capacity(64 * 1024, File::open(path)?)
                .bytes()
                .peekable(),
            cancellation: cancellation.clone(),
            count: 0,
        })
    }

    fn next(&mut self) -> Result<Option<u8>> {
        self.count += 1;
        if self.count.is_multiple_of(65536) {
            common::check_cancel(&self.cancellation)?;
        }
        self.bytes
            .next()
            .transpose()
            .context("cannot read SQL artifact")
    }

    fn peek(&mut self) -> Result<Option<u8>> {
        match self.bytes.peek() {
            Some(Ok(value)) => Ok(Some(*value)),
            Some(Err(_)) => self.next(),
            None => Ok(None),
        }
    }

    fn line_comment(&mut self) -> Result<()> {
        while let Some(byte) = self.next()? {
            if byte == b'\n' {
                break;
            }
        }
        Ok(())
    }

    fn block_comment(&mut self) -> Result<()> {
        while let Some(byte) = self.next()? {
            if byte == b'*' && self.peek()? == Some(b'/') {
                self.next()?;
                return Ok(());
            }
        }
        bail!("SQL artifact has an unterminated comment")
    }

    fn quoted(&mut self, quote: u8, retain: bool, check_mode: bool) -> Result<String> {
        let mut value = Vec::new();
        let mut no_backslash_window = VecDeque::new();
        while let Some(byte) = self.next()? {
            if byte == quote {
                if self.peek()? == Some(quote) {
                    self.next()?;
                } else {
                    return String::from_utf8(value).context("SQL identifier must be UTF-8");
                }
            } else if byte == b'\\' {
                let escaped = self.next()?.context("SQL artifact ends inside an escape")?;
                if retain {
                    value.push(escaped);
                    ensure!(value.len() <= 1024, "SQL artifact identifier is too long");
                }
                continue;
            }
            if retain {
                value.push(byte);
                ensure!(value.len() <= 1024, "SQL artifact identifier is too long");
            } else if check_mode {
                no_backslash_window.push_back(byte.to_ascii_lowercase());
                if no_backslash_window.len() > b"no_backslash_escapes".len() {
                    no_backslash_window.pop_front();
                }
                ensure!(
                    no_backslash_window
                        .iter()
                        .copied()
                        .ne(b"no_backslash_escapes".iter().copied()),
                    "SQL artifacts using NO_BACKSLASH_ESCAPES need manual review before native restoration"
                );
            }
        }
        bail!("SQL artifact has an unterminated quoted value")
    }
}

struct SqlGuard<'a> {
    source: &'a str,
    destination: Option<&'a str>,
    words: VecDeque<String>,
    previous: Option<String>,
    object_expected: bool,
    object_candidate: Option<String>,
    previous_dot: bool,
}

impl<'a> SqlGuard<'a> {
    fn new(source: &'a str, destination: Option<&'a str>) -> Self {
        Self {
            source,
            destination,
            words: VecDeque::new(),
            previous: None,
            object_expected: false,
            object_candidate: None,
            previous_dot: false,
        }
    }

    fn word(&mut self, word: String, quoted: bool) -> Result<()> {
        let lowered = word.to_ascii_lowercase();
        if !quoted {
            ensure!(
                !matches!(
                    lowered.as_str(),
                    "use"
                        | "source"
                        | "connect"
                        | "system"
                        | "tee"
                        | "pager"
                        | "quit"
                        | "shutdown"
                        | "reset"
                        | "flush"
                        | "grant"
                        | "revoke"
                        | "install"
                        | "uninstall"
                        | "rename"
                        | "prepare"
                        | "execute"
                        | "call"
                        | "soname"
                        | "outfile"
                        | "dumpfile"
                        | "tablespace"
                ),
                "SQL artifact contains an unsupported routing, external-file, dynamic, or administrative statement"
            );
            let previous = self.words.back().map(String::as_str);
            ensure!(
                !matches!(
                    (previous, lowered.as_str()),
                    (
                        Some("create" | "drop" | "alter"),
                        "database" | "schema" | "user" | "server"
                    ) | (Some("load"), "data" | "xml")
                        | (Some("set"), "global")
                        | (Some("data" | "index"), "directory")
                ),
                "SQL artifact contains a database/global/file operation; restoration is restricted to a new target"
            );
            self.words.push_back(lowered.clone());
            if self.words.len() > 4 {
                self.words.pop_front();
            }
        }
        if self.object_expected
            && (quoted || !matches!(lowered.as_str(), "if" | "not" | "exists" | "only"))
        {
            self.object_candidate = Some(word.clone());
            self.object_expected = false;
        }
        if !quoted
            && matches!(
                lowered.as_str(),
                "from"
                    | "join"
                    | "into"
                    | "update"
                    | "references"
                    | "table"
                    | "view"
                    | "trigger"
                    | "procedure"
                    | "function"
                    | "event"
            )
        {
            self.object_expected = true;
            self.object_candidate = None;
        }
        if self.previous_dot {
            self.object_candidate = None;
        }
        self.previous = Some(word);
        self.previous_dot = false;
        Ok(())
    }

    fn mode_literal(&self) -> bool {
        self.words.iter().any(|word| word == "set")
            && self.words.back().is_some_and(|word| word == "sql_mode")
    }

    fn symbol(&mut self, byte: u8) -> Result<()> {
        if byte == b'.' {
            if let Some(previous) = &self.previous {
                let destination = self
                    .destination
                    .is_some_and(|database| database.eq_ignore_ascii_case(previous));
                ensure!(
                    self.destination.is_none()
                        || destination
                        || !previous.eq_ignore_ascii_case(self.source),
                    "SQL artifact qualifies objects with the original database; cross-database references require manual migration"
                );
                ensure!(
                    self.destination.is_none() || self.object_candidate.is_none() || destination,
                    "SQL artifact uses a qualified object outside the new destination database"
                );
                ensure!(
                    !previous.eq_ignore_ascii_case("global"),
                    "SQL artifact attempts to modify global server state"
                );
            }
            self.previous_dot = true;
        } else if !matches!(byte, b'@' | b'=') {
            self.object_candidate = None;
            self.previous = None;
            self.previous_dot = false;
            if byte == b';' {
                self.words.clear();
                self.object_expected = false;
            }
        }
        Ok(())
    }
}

fn scan_sql(
    path: &Path,
    metadata: &DumpMetadata,
    destination: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<()> {
    common::check_cancel(cancellation)?;
    let mut input = SqlBytes::new(path, cancellation)?;
    let mut guard = SqlGuard::new(&metadata.database, destination);
    while let Some(byte) = input.next()? {
        match byte {
            byte if byte.is_ascii_whitespace() => {}
            b'\'' => {
                input.quoted(b'\'', false, guard.mode_literal())?;
                guard.symbol(b'?')?;
            }
            b'"' if guard.mode_literal() => {
                input.quoted(byte, false, true)?;
                guard.symbol(b'?')?;
            }
            b'`' | b'"' => guard.word(input.quoted(byte, true, false)?, true)?,
            b'0' if matches!(input.peek()?, Some(b'x' | b'X')) => {
                input.next()?;
                while input.peek()?.is_some_and(|byte| byte.is_ascii_hexdigit()) {
                    input.next()?;
                }
                guard.symbol(b'?')?;
            }
            b'#' => input.line_comment()?,
            b'-' if input.peek()? == Some(b'-') => {
                input.next()?;
                if input.peek()?.is_some_and(|byte| byte.is_ascii_whitespace()) {
                    input.line_comment()?;
                } else {
                    guard.symbol(b'-')?;
                }
            }
            b'/' if input.peek()? == Some(b'*') => {
                input.next()?;
                let maria = input.peek()? == Some(b'M');
                if maria {
                    input.next()?;
                }
                if input.peek()? == Some(b'!') {
                    input.next()?;
                    while input.peek()?.is_some_and(|byte| byte.is_ascii_digit()) {
                        input.next()?;
                    }
                    if maria && input.peek()? == Some(b'\\') {
                        let mut sandbox = Vec::new();
                        while let Some(byte) = input.next()? {
                            sandbox.push(byte);
                            ensure!(sandbox.len() <= 128, "invalid MariaDB sandbox directive");
                            if sandbox.ends_with(b"*/") {
                                break;
                            }
                        }
                        ensure!(
                            sandbox == b"\\- enable the sandbox mode */",
                            "unsupported MariaDB executable-comment directive"
                        );
                    }
                } else {
                    input.block_comment()?;
                }
            }
            b'*' if input.peek()? == Some(b'/') => {
                input.next()?;
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' || byte >= 128 => {
                let mut word = vec![byte];
                while input.peek()?.is_some_and(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') || byte >= 128
                }) {
                    word.push(input.next()?.context("SQL token ended unexpectedly")?);
                    ensure!(word.len() <= 1024, "SQL artifact token is too long");
                }
                let word = String::from_utf8(word).context("SQL artifact token must be UTF-8")?;
                if word.eq_ignore_ascii_case("delimiter") {
                    input.line_comment()?;
                    guard.symbol(b';')?;
                } else {
                    guard.word(word, false)?;
                }
            }
            b'\\' => bail!("SQL artifact contains an unsupported client command"),
            byte => guard.symbol(byte)?,
        }
    }
    common::check_cancel(cancellation)?;
    Ok(())
}

async fn validate_dump(
    path: &Path,
    engine: DatabaseKind,
    destination: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<(u32, u32, u32)> {
    let path = path.to_path_buf();
    let destination = destination.map(str::to_owned);
    let token = cancellation.clone();
    tokio::task::spawn_blocking(move || {
        let metadata = dump_metadata(&path, engine)?;
        if destination.is_some() {
            scan_sql(&path, &metadata, destination.as_deref(), &token)?;
        }
        common::check_cancel(&token)?;
        Ok(metadata.version)
    })
    .await
    .context("SQL artifact validation worker failed")?
}

pub(super) async fn plan(
    spec: &DataJobSpec,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<NativePlan> {
    let engine = source
        .or(destination)
        .context("native endpoint is missing")?
        .profile
        .kind;
    let redactor = Redactor::new(&[source, destination]);
    let (dump_name, restore_name) = names(engine);
    let mut tools = Vec::new();
    if source.is_some() {
        tools.push(
            common::tool(
                dump_name,
                spec.tools.dump.as_deref(),
                cancellation,
                &redactor,
            )
            .await?,
        );
    }
    if destination.is_some() {
        tools.push(
            common::tool(
                restore_name,
                spec.tools.restore.as_deref(),
                cancellation,
                &redactor,
            )
            .await?,
        );
    }
    for tool in &tools {
        let version = check_family(&tool.version, engine)?;
        let minimum = if engine == DatabaseKind::MariaDb {
            (10, 5, 0)
        } else {
            (8, 0, 0)
        };
        ensure!(
            version >= minimum,
            "native MySQL clients require 8.0+; MariaDB clients require 10.5+"
        );
        ensure!(
            tool.version.to_ascii_lowercase().starts_with(&tool.name),
            "native executable reported an unexpected dump/restore client identity"
        );
    }
    let input_version = if let Some(source) = source {
        tls_arguments(source)?;
        let source_version = source_version(source, connector, cancellation).await?;
        let dump = tools
            .iter()
            .find(|tool| tool.name == dump_name)
            .context("native dump client is missing")?;
        let dump_version = check_family(&dump.version, engine)?;
        ensure!(
            (dump_version.0, dump_version.1) == (source_version.0, source_version.1),
            "native dump client and source server must use the same major.minor series"
        );
        let route = route(connector, source, cancellation).await?;
        let _ = client(dump, source, &route)?;
        source_version
    } else {
        validate_dump(
            &spec.artifact,
            engine,
            destination.map(|endpoint| endpoint.database.as_str()),
            cancellation,
        )
        .await?
    };
    if let Some(destination) = destination {
        tls_arguments(destination)?;
        let destination_version = destination_version(destination, connector, cancellation).await?;
        let restore = tools
            .iter()
            .find(|tool| tool.name == restore_name)
            .context("native restore client is missing")?;
        let restore_version = check_family(&restore.version, engine)?;
        ensure!(
            (destination_version.0, destination_version.1) == (input_version.0, input_version.1)
                && destination_version >= input_version
                && (restore_version.0, restore_version.1)
                    == (destination_version.0, destination_version.1),
            "native SQL restore requires the same major.minor engine series and a destination patch version at least as new as the source"
        );
        let route = route(connector, destination, cancellation).await?;
        let _ = client(restore, destination, &route)?;
    }
    let steps = match spec.operation {
        DataOperation::Backup => vec![
            format!("Run {dump_name} for one complete database with --single-transaction --quick --routines --events --triggers"),
            "Check the native SQL signature/completion marker, hash, and publish exclusively".into(),
        ],
        DataOperation::Restore => vec![
            "Pin the native SQL dump and validate its signature, version, completion marker, and database-routing guards".into(),
            "Recheck destination absence and CREATE DATABASE without IF NOT EXISTS".into(),
            format!("Stream SQL to {restore_name} stdin using --binary-mode --batch --skip-reconnect"),
        ],
        DataOperation::Copy => vec![
            format!("Create and exclusively publish a validated full-database {dump_name} SQL dump"),
            "Recheck destination absence and create a new database".into(),
            format!("Stream the dump to {restore_name}; retain the completed backup if restoration fails"),
        ],
    };
    let mut warnings = vec![
        "A single-transaction dump is consistent for transactional tables only; nontransactional tables and concurrent DDL/routine/event changes require an externally coordinated quiet period.".into(),
        "Routines, events, and triggers are requested explicitly; their definers and privileges may prevent restore. Accounts, grants, GTID state, replication configuration, and other databases are excluded.".into(),
        "SQL dump verification checks its native header and completion marker; restoration additionally checks routing restrictions. Neither check proves complete SQL syntax or data parity.".into(),
        "Qualified references outside the new database, dynamic SQL, administrative/file commands, and NO_BACKSLASH_ESCAPES artifacts require manual migration.".into(),
    ];
    if spec.operation.writes_database() {
        warnings.extend([
            "The new database uses destination defaults for database-level character set/collation. Restore-client success does not prove full schema/data parity.".into(),
            "Restored events and triggers retain their behavior and definers; enabled events may run on a destination with its event scheduler enabled.".into(),
            "Failure or cancellation after database creation leaves a possibly partial target for inspection; the provider never drops it.".into(),
            "Restore only trusted native SQL artifacts; the routing guard is not a general SQL security sandbox.".into(),
        ]);
    }
    if engine == DatabaseKind::MariaDb
        && [source, destination]
            .into_iter()
            .flatten()
            .any(|endpoint| endpoint.profile.ssl_mode == SslMode::Prefer)
    {
        warnings.push("MariaDB prefer-TLS keeps the client's opportunistic TLS behavior and does not verify server certificates; it does not guarantee encryption.".into());
    }
    Ok(NativePlan {
        engine,
        steps,
        warnings,
        tools,
    })
}

pub(super) async fn execute(
    spec: &DataJobSpec,
    plan: &NativePlan,
    endpoints: (Option<&ResolvedEndpoint>, Option<&ResolvedEndpoint>),
    pinned: Option<&PinnedInput>,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
    progress: &ProgressSink,
) -> Result<NativeOutcome> {
    let (source, destination) = endpoints;
    let engine = plan.engine;
    let (dump_name, restore_name) = names(engine);
    let redactor = Redactor::new(&[source, destination]);
    let mut outcome = NativeOutcome {
        warnings: plan.warnings.clone(),
        ..NativeOutcome::default()
    };
    let artifact = if let Some(source) = source {
        let output = common::output_path(&spec.artifact)?;
        let stage = StagedArtifact::new(&output)?;
        let partial = stage.path().to_path_buf();
        let dumped = async {
            let dump = plan
                .tools
                .iter()
                .find(|tool| tool.name == dump_name)
                .context("native dump client is missing")?;
            let route = route(connector, source, cancellation).await?;
            let mut process = client(dump, source, &route)?;
            process
                .command
                .args(dump_arguments(engine, &source.database))
                .stdout(Stdio::from(stage.writer()?));
            common::progress(
                progress,
                DataJobStatus::Running,
                format!("Running {dump_name} full database backup"),
                None,
            );
            let result = process
                .run(cancellation, &redactor, Some(&route), None)
                .await?;
            common::progress(
                progress,
                DataJobStatus::Verifying,
                "Validating native SQL dump signature and completion",
                None,
            );
            validate_dump(stage.path(), engine, None, cancellation).await?;
            let fingerprint = common::fingerprint(stage.path(), cancellation).await?;
            common::check_cancel(cancellation)?;
            stage.publish(&output)?;
            Ok::<_, anyhow::Error>((fingerprint, result.stderr))
        }
        .await
        .with_context(|| {
            format!(
                "native SQL backup failed; incomplete artifact retained at {}",
                partial.display()
            )
        })?;
        outcome.bytes = Some(dumped.0.0);
        outcome.sha256 = Some(dumped.0.1);
        outcome.artifact = Some(output.clone());
        outcome
            .verification
            .push("Native SQL dump signature and completion marker validated".into());
        if !dumped.1.is_empty() {
            outcome
                .warnings
                .push(format!("{dump_name} diagnostics: {}", dumped.1));
        }
        common::backup_completed(progress, &outcome);
        output
    } else {
        let input = pinned.context("SQL restore input was not pinned")?;
        validate_dump(
            input.path(),
            engine,
            destination.map(|endpoint| endpoint.database.as_str()),
            cancellation,
        )
        .await?;
        outcome.bytes = Some(input.bytes);
        outcome.sha256 = Some(input.sha256.clone());
        outcome.artifact = Some(spec.artifact.clone());
        input.path().to_path_buf()
    };
    if spec.operation.writes_database() {
        common::check_cancel(cancellation)?;
        let destination = destination.context("native SQL restore destination is missing")?;
        let restored = async {
            let copy_input = if spec.operation == DataOperation::Copy {
                let input = PinnedInput::new(&artifact, cancellation).await?;
                ensure!(
                    Some(input.bytes) == outcome.bytes && Some(&input.sha256) == outcome.sha256.as_ref(),
                    "completed SQL backup changed before restore"
                );
                Some(input)
            } else {
                None
            };
            let restore_path = copy_input.as_ref().map_or(artifact.as_path(), PinnedInput::path);
            let input_version = validate_dump(restore_path, engine, Some(&destination.database), cancellation).await?;
            outcome.verification.push("Native SQL restore input passed database-routing restrictions".into());
            let restore = plan
                .tools
                .iter()
                .find(|tool| tool.name == restore_name)
                .context("native restore client is missing")?;
            let route = route(connector, destination, cancellation).await?;
            let mut process = client(restore, destination, &route)?;
            process
                .command
                .args(restore_arguments(&destination.database))
                .stdin(Stdio::from(common::reader(restore_path)?));
            let connection = common::connect(connector, destination, None, cancellation).await?;
            let creation = async {
                ensure_destination_absent(&connection, &destination.database, cancellation).await?;
                let destination_version = server_version(&connection, engine, cancellation).await?;
                ensure!(
                    (input_version.0, input_version.1) == (destination_version.0, destination_version.1)
                        && destination_version >= input_version,
                    "destination MySQL/MariaDB version changed and is incompatible with the SQL dump"
                );
                common::check_cancel(cancellation)?;
                common::progress(
                    progress,
                    DataJobStatus::Running,
                    "Creating new MySQL/MariaDB database",
                    outcome.bytes,
                );
                let statement = format!(
                    "CREATE DATABASE {}",
                    common::quote_identifier(&destination.database, true)?
                );
                common::sql(&connection, &statement, cancellation).await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            connection.close().await;
            creation?;
            outcome.target_created = true;
            common::progress(
                progress,
                DataJobStatus::Running,
                format!("Streaming native SQL into {restore_name}"),
                outcome.bytes,
            );
            let result = process
                .run(cancellation, &redactor, Some(&route), None)
                .await?;
            common::progress(
                progress,
                DataJobStatus::Verifying,
                "SQL restore client completed; data parity was not checked",
                outcome.bytes,
            );
            if !result.stderr.is_empty() {
                outcome
                    .warnings
                    .push(format!("{restore_name} diagnostics: {}", result.stderr));
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        restored.with_context(|| format!(
            "native SQL restore failed; the new database may be partial and was not dropped; completed artifact kept at {}",
            spec.artifact.display()
        ))?;
        outcome.verification.push(format!(
            "{restore_name} restore command succeeded; full schema/data parity was not checked"
        ));
    }
    common::check_cancel(cancellation)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::import_connection_url;

    fn sql_artifact(sql: &str) -> NamedSql {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), format!(
            "-- MySQL dump 10.13 Distrib 8.0.40, for Linux\n-- Host: localhost    Database: original\n-- Server version 8.0.40\n{sql}\n-- Dump completed on 2026-11-16\n"
        )).unwrap();
        NamedSql(file)
    }

    struct NamedSql(tempfile::NamedTempFile);

    #[test]
    fn dump_and_restore_commands_are_distinct_and_never_select_original_on_restore() {
        assert_eq!(names(DatabaseKind::MySql), ("mysqldump", "mysql"));
        assert_eq!(names(DatabaseKind::MariaDb), ("mariadb-dump", "mariadb"));
        let arguments = dump_arguments(DatabaseKind::MySql, "--dangerous");
        for required in [
            "--single-transaction",
            "--quick",
            "--routines",
            "--events",
            "--triggers",
            "--set-gtid-purged=OFF",
        ] {
            assert!(arguments.contains(&required.into()));
        }
        assert!(!arguments.contains(&"--databases".into()));
        assert_eq!(&arguments[arguments.len() - 2..], ["--", "--dangerous"]);
        assert!(restore_arguments("new target").contains(&"--database=new target".into()));
        assert!(
            !restore_arguments("new target")
                .iter()
                .any(|argument| argument.contains("original"))
        );
    }

    #[test]
    fn connection_timeout_is_only_passed_to_sql_restore_clients() {
        for scheme in ["mysql", "mariadb"] {
            let profile = import_connection_url(
                &format!("{scheme}://user@localhost/original?sslMode=disable"),
                None,
            )
            .unwrap()
            .profile;
            let endpoint = ResolvedEndpoint {
                profile,
                database: "original".into(),
                password: None,
            };
            let (dump_name, restore_name) = names(endpoint.profile.kind);
            for name in [dump_name, restore_name] {
                let tool = NativeToolVersion {
                    name: name.into(),
                    path: "/not/executed".into(),
                    version: "not needed for construction".into(),
                };
                let process = client(&tool, &endpoint, &ConnectionRoute::direct()).unwrap();
                let timeout = process
                    .command
                    .as_std()
                    .get_args()
                    .any(|argument| argument == "--connect-timeout=10");
                assert_eq!(timeout, name == restore_name, "{name}");
            }
        }
    }

    #[test]
    fn mysql_password_never_enters_arguments_or_inherited_environment() {
        let profile =
            import_connection_url("mysql://user@localhost/original?sslMode=disable", None)
                .unwrap()
                .profile;
        let endpoint = ResolvedEndpoint {
            profile,
            database: "original".into(),
            password: Some(secrecy::SecretString::from("hidden\"\\\npassword")),
        };
        let tool = NativeToolVersion {
            name: "mysqldump".into(),
            path: "/not/executed".into(),
            version: "mysqldump Ver 8.0.40".into(),
        };
        let process = client(&tool, &endpoint, &ConnectionRoute::direct()).unwrap();
        let command = process.command.as_std();
        let arguments: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert!(arguments[0].starts_with("--defaults-file="));
        assert!(
            arguments
                .iter()
                .all(|argument| !argument.contains("hidden"))
        );
        assert!(
            command
                .get_envs()
                .all(|(name, _)| name != "MYSQL_PWD" && name != "MYSQL_HOST")
        );
        let defaults = arguments[0].strip_prefix("--defaults-file=").unwrap();
        assert_eq!(
            std::fs::read_to_string(defaults).unwrap(),
            "[client]\npassword=\"hidden\\\"\\\\\\npassword\"\n"
        );
    }

    #[test]
    fn routing_guard_accepts_data_and_versioned_routine_definitions() {
        let file = sql_artifact(
            "CREATE TABLE `items` (`value` text);\nINSERT INTO `items` VALUES ('USE original; \\\\ literal');\nDELIMITER ;;\n/*!50003 CREATE*/ /*!50017 DEFINER=`user`@`host`*/ /*!50003 PROCEDURE `routine`() BEGIN SELECT `items`.`value` FROM `items`; END */;;\nDELIMITER ;",
        );
        let metadata = dump_metadata(file.0.path(), DatabaseKind::MySql).unwrap();
        scan_sql(
            file.0.path(),
            &metadata,
            Some("new_target"),
            &CancellationToken::new(),
        )
        .unwrap();
    }

    #[test]
    fn routing_guard_streams_large_hex_blobs_and_preserves_sql_mode_text_in_data() {
        let file = sql_artifact(&format!(
            "CREATE TABLE `items` (`payload` LONGBLOB, `label` TEXT);\nINSERT INTO `items` VALUES (0x{}, 'NO_BACKSLASH_ESCAPES');",
            "abcdef0123456789".repeat(16 * 1024)
        ));
        let metadata = dump_metadata(file.0.path(), DatabaseKind::MySql).unwrap();
        scan_sql(
            file.0.path(),
            &metadata,
            Some("new_target"),
            &CancellationToken::new(),
        )
        .unwrap();
    }

    #[test]
    fn routing_guard_rejects_hidden_database_selection_and_external_writes() {
        for sql in [
            "USE `original`;",
            "/*!50000 USE `original` */;",
            "DROP /* split */ DATABASE `original`;",
            "CREATE SCHEMA `other`;",
            "INSERT INTO `original`.`items` VALUES (1);",
            "CREATE VIEW `v` AS SELECT * FROM `other`.`items`;",
            "SELECT * FROM `items` INTO OUTFILE '/tmp/output';",
            "SET @@GLOBAL.gtid_purged='x';",
            "PREPARE statement FROM 'USE original';",
            "SET SQL_MODE='NO_BACKSLASH_ESCAPES';",
            "SET SQL_MODE=\"NO_BACKSLASH_ESCAPES\";",
            "\\. /tmp/another.sql",
        ] {
            let file = sql_artifact(sql);
            let metadata = dump_metadata(file.0.path(), DatabaseKind::MySql).unwrap();
            assert!(
                scan_sql(
                    file.0.path(),
                    &metadata,
                    Some("new_target"),
                    &CancellationToken::new()
                )
                .is_err(),
                "accepted: {sql}"
            );
        }
    }

    #[test]
    fn incomplete_and_cross_engine_sql_artifacts_are_rejected() {
        let file = sql_artifact("CREATE TABLE `items` (`id` int);");
        assert!(dump_metadata(file.0.path(), DatabaseKind::MariaDb).is_err());
        std::fs::write(file.0.path(), b"-- MySQL dump 10.13 Distrib 8.0.40\n-- Host: localhost Database: original\n-- Server version 8.0.40\n").unwrap();
        assert!(dump_metadata(file.0.path(), DatabaseKind::MySql).is_err());
    }

    #[tokio::test]
    async fn backups_preserve_native_programs_while_relocation_requires_routing_review() {
        let file = sql_artifact(
            "CREATE VIEW `v` AS SELECT * FROM `original`.`items`;\nDELIMITER ;;\nCREATE PROCEDURE `p`() BEGIN PREPARE stmt FROM 'SELECT 1'; END;;\nDELIMITER ;",
        );
        let token = CancellationToken::new();
        validate_dump(file.0.path(), DatabaseKind::MySql, None, &token)
            .await
            .unwrap();
        assert!(
            validate_dump(
                file.0.path(),
                DatabaseKind::MySql,
                Some("new_target"),
                &token
            )
            .await
            .is_err()
        );
    }
}
