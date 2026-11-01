//! Opt-in, local insecure CockroachDB tests. The relay changes TCP delivery, never SQL replies.
use lazydb::{
    action::{Action, Command},
    db::{
        ErrorCategory,
        cockroach::{CockroachAdapter, CockroachTransactionBackend},
        query::QueryOutcome,
        transaction::{RelationMutationCategory, TransactionBackend, TransactionError},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::execution_target::ExecutionTarget,
    persistence::{profiles::ProfileStore, secrets::NativeSecretStore},
    profile::{ConnectionProfile, DatabaseKind, SslMode, import_connection_url},
    runtime::Runtime,
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use uuid::Uuid;

const IO_TIMEOUT: Duration = Duration::from_secs(15);

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    timeout(IO_TIMEOUT, future)
        .await
        .expect("database/relay operation timed out")
}

fn local_profile() -> Option<ConnectionProfile> {
    let url = match std::env::var("LAZYDB_TEST_COCKROACHDB_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ if std::env::var_os("LAZYDB_REQUIRE_DATABASE_TESTS").is_some() => {
            panic!("LAZYDB_TEST_COCKROACHDB_URL is required for CockroachDB commit fault tests")
        }
        _ => {
            eprintln!("SKIP: LAZYDB_TEST_COCKROACHDB_URL is not set");
            return None;
        }
    };
    // Do not include the URL in diagnostics: it may contain credentials.
    let imported = import_connection_url(&url, None)
        .unwrap_or_else(|_| panic!("invalid CockroachDB commit fault test URL"));
    let mut profile = imported.profile;
    assert!(matches!(
        profile.kind,
        DatabaseKind::CockroachDb | DatabaseKind::Postgres
    ));
    assert!(
        matches!(
            profile.host.as_deref(),
            Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
        ),
        "commit fault injection requires a literal loopback host or localhost"
    );
    assert_eq!(
        profile.ssl_mode,
        SslMode::Disable,
        "commit fault injection requires explicit sslmode=disable"
    );
    assert!(
        imported.transient_password.is_none(),
        "commit fault injection requires a password-free insecure local server"
    );
    assert!(
        profile
            .pg_options
            .as_ref()
            .is_none_or(|options| options.options.is_none()),
        "commit fault injection does not allow startup routing options"
    );
    profile.kind = DatabaseKind::CockroachDb;
    Some(profile)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
    PassThrough,
    BeforeCommit,
    AfterCommitSuccess,
}

#[derive(Default)]
struct Evidence {
    sql: Vec<String>,
    commits_forwarded: usize,
    commits_confirmed: usize,
    faults: usize,
}

struct Relay {
    port: u16,
    evidence: Arc<Mutex<Evidence>>,
    task: JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        // Dropping the listener task also drops its JoinSet and all accepted sockets.
        self.task.abort();
    }
}

impl Relay {
    async fn start(profile: &ConnectionProfile, fault: Fault) -> Self {
        let upstream_ip = match profile.host.as_deref() {
            Some("::1" | "[::1]") => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
        };
        let upstream = SocketAddr::new(upstream_ip, profile.port.unwrap_or(26257));
        let listener = bounded(TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let evidence = Arc::new(Mutex::new(Evidence::default()));
        let shared = evidence.clone();
        let task = tokio::spawn(async move {
            let mut sessions = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (client, _) = accepted.unwrap();
                        let evidence = shared.clone();
                        sessions.spawn(async move {
                            bounded(relay_session(client, upstream, fault, evidence)).await
                        });
                    }
                    result = sessions.join_next(), if !sessions.is_empty() => {
                        // A panic in a relay must fail the owning task, rather than disappear.
                        let result = result.unwrap().unwrap();
                        if let Err(error) = result {
                            assert!(matches!(error.kind(), io::ErrorKind::UnexpectedEof
                                | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe),
                                "unexpected relay I/O error: {:?}", error.kind());
                        }
                    }
                }
            }
        });
        Self {
            port,
            evidence,
            task,
        }
    }

    fn profile(&self, original: &ConnectionProfile) -> ConnectionProfile {
        let mut profile = original.clone();
        profile.host = Some("127.0.0.1".into());
        profile.port = Some(self.port);
        profile
    }

    fn assert_delivery(&self, fault: Fault, marker: &str) {
        assert!(!self.task.is_finished(), "relay task failed");
        let evidence = self.evidence.lock().unwrap();
        assert_eq!(
            evidence.sql.iter().filter(|sql| is_commit(sql)).count(),
            1,
            "exactly one COMMIT attempt; no automatic replay"
        );
        assert_eq!(
            evidence
                .sql
                .iter()
                .filter(|sql| sql.contains(marker))
                .count(),
            1,
            "the mutation must not be replayed"
        );
        assert_eq!(evidence.faults, 1);
        assert_eq!(
            evidence.commits_forwarded,
            usize::from(fault == Fault::AfterCommitSuccess)
        );
        assert_eq!(
            evidence.commits_confirmed,
            usize::from(fault == Fault::AfterCommitSuccess)
        );
    }
}

fn is_commit(sql: &str) -> bool {
    sql.trim()
        .trim_end_matches(';')
        .trim()
        .eq_ignore_ascii_case("COMMIT")
}

// PostgreSQL v3 frames have a tag followed by a big-endian length (including itself).
async fn frame(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Option<Vec<u8>>> {
    let tag = match reader.read_u8().await {
        Ok(tag) => tag,
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    };
    let length = reader.read_u32().await? as usize;
    if !(4..=1024 * 1024).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid pgwire frame length",
        ));
    }
    let mut bytes = vec![0; length + 1];
    bytes[0] = tag;
    bytes[1..5].copy_from_slice(&(length as u32).to_be_bytes());
    reader.read_exact(&mut bytes[5..]).await?;
    Ok(Some(bytes))
}

async fn relay_session(
    mut client: TcpStream,
    upstream: SocketAddr,
    fault: Fault,
    evidence: Arc<Mutex<Evidence>>,
) -> io::Result<()> {
    let mut server = TcpStream::connect(upstream).await?;
    // Startup is the only untagged message. Reject SSLRequest and all other protocols.
    let length = client.read_u32().await? as usize;
    if !(8..=64 * 1024).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid startup length",
        ));
    }
    let mut startup = vec![0; length];
    startup[..4].copy_from_slice(&(length as u32).to_be_bytes());
    client.read_exact(&mut startup[4..]).await?;
    assert_eq!(
        &startup[4..8],
        &196608_u32.to_be_bytes(),
        "only plaintext PostgreSQL v3 is allowed"
    );
    server.write_all(&startup).await?;
    let (mut client_read, mut client_write) = client.split();
    let (mut server_read, mut server_write) = server.split();
    let (commit_sent, mut commit_received) = mpsc::unbounded_channel();
    let client_to_server = async {
        while let Some(bytes) = frame(&mut client_read).await? {
            if bytes[0] == b'Q' {
                let sql = std::str::from_utf8(&bytes[5..bytes.len() - 1]).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid query encoding")
                })?;
                evidence.lock().unwrap().sql.push(sql.to_owned());
                if is_commit(sql) {
                    if fault == Fault::BeforeCommit {
                        evidence.lock().unwrap().faults += 1;
                        return Ok::<_, io::Error>(()); // Drop both TCP sockets without forwarding COMMIT.
                    }
                    evidence.lock().unwrap().commits_forwarded += 1;
                    if fault == Fault::AfterCommitSuccess {
                        commit_sent.send(()).unwrap();
                    }
                }
            }
            server_write.write_all(&bytes).await?;
        }
        Ok(())
    };
    let server_to_client = async {
        let mut dropping_commit = false;
        let mut committed = false;
        while let Some(bytes) = frame(&mut server_read).await? {
            // SQLx drains each preceding operation before issuing the simple-query COMMIT.
            dropping_commit |= commit_received.try_recv().is_ok();
            if dropping_commit {
                assert_ne!(
                    bytes[0], b'E',
                    "server must really accept COMMIT before response loss"
                );
                if bytes[0] == b'C' {
                    assert_eq!(&bytes[5..], b"COMMIT\0");
                    committed = true;
                }
                if bytes[0] == b'Z' {
                    assert!(
                        committed,
                        "ReadyForQuery alone does not prove a successful commit"
                    );
                    assert_eq!(&bytes[5..], b"I", "server must have left the transaction");
                    let mut evidence = evidence.lock().unwrap();
                    evidence.commits_confirmed += 1;
                    evidence.faults += 1;
                    return Ok::<_, io::Error>(()); // All commit response frames were withheld.
                }
            } else {
                client_write.write_all(&bytes).await?;
            }
        }
        Ok(())
    };
    // Neither direction is repeatedly cancelled mid-frame. Completion drops both sockets.
    tokio::select! {
        result = client_to_server => result,
        result = server_to_client => result,
    }
}

struct Fixture {
    profile: ConnectionProfile,
    observer: CockroachAdapter,
    table: String,
}

impl Fixture {
    async fn new() -> Option<Self> {
        let profile = local_profile()?;
        let observer = bounded(CockroachAdapter::connect(&profile, None))
            .await
            .unwrap();
        let table = format!("public.lazydb_commit_fault_{}", Uuid::new_v4().simple());
        bounded(observer.execute(&format!(
            "CREATE TABLE {table} (id INT PRIMARY KEY, value INT NOT NULL)"
        )))
        .await
        .unwrap();
        Some(Self {
            profile,
            observer,
            table,
        })
    }

    async fn rows(&self) -> Vec<Vec<CellValue>> {
        // This adapter connects directly to the server, independently of the relay/pinned session.
        bounded(
            self.observer
                .execute(&format!("SELECT id, value FROM {} ORDER BY id", self.table)),
        )
        .await
        .unwrap()
        .result_sets
        .remove(0)
        .rows
    }

    async fn finish(self) {
        bounded(self.observer.execute(&format!("DROP TABLE {}", self.table)))
            .await
            .unwrap();
        bounded(self.observer.close()).await;
    }
}

fn assert_unknown(error: &TransactionError) {
    let diagnostic = error
        .relation_diagnostic()
        .expect("structured commit diagnostic");
    assert_eq!(diagnostic.operation, "commit");
    assert_eq!(
        diagnostic.category,
        RelationMutationCategory::ConnectionUnknown
    );
    assert_eq!(error.history_snapshot().category, ErrorCategory::Network);
    assert!(diagnostic.message.contains("commit outcome is unknown"));
    assert!(
        diagnostic
            .message
            .contains("verify the result before starting another transaction")
    );
}

async fn backend_disconnect(fault: Fault) {
    let Some(fixture) = Fixture::new().await else {
        return;
    };
    let relay = Relay::start(&fixture.profile, fault).await;
    let adapter = bounded(CockroachAdapter::connect(
        &relay.profile(&fixture.profile),
        None,
    ))
    .await
    .unwrap();
    let mut backend = bounded(adapter.transaction_backend()).await.unwrap();
    bounded(backend.begin()).await.unwrap();
    let insert = format!("INSERT INTO {} VALUES (1, 7)", fixture.table);
    bounded(backend.execute(&insert)).await.unwrap();
    let error = bounded(backend.commit()).await.unwrap_err();
    assert_unknown(&error);
    assert!(
        !backend.commit_failure_requires_rollback(&error),
        "unknown outcome requires quarantine"
    );
    // Mirror runtime quarantine cleanup; the uncertain pinned socket must never return to its pool.
    bounded(backend.force_close()).await.unwrap();
    let expected = if fault == Fault::BeforeCommit {
        vec![]
    } else {
        vec![vec![CellValue::Integer(1), CellValue::Integer(7)]]
    };
    assert_eq!(fixture.rows().await, expected);
    // Exercise the same pool after quarantine; reconnection must not replay the failed transaction.
    bounded(adapter.execute("SELECT 1")).await.unwrap();
    bounded(adapter.close()).await;
    assert_eq!(fixture.rows().await, expected);
    relay.assert_delivery(fault, &insert);
    fixture.finish().await;
}

#[tokio::test]
async fn commit_disconnect_before_forwarding_is_unknown_but_does_not_commit_or_replay() {
    backend_disconnect(Fault::BeforeCommit).await;
}

#[tokio::test]
async fn commit_success_with_lost_response_is_unknown_but_commits_once_without_replay() {
    backend_disconnect(Fault::AfterCommitSuccess).await;
}

// A higher-priority writer aborts the pinned transaction using ordinary SQL permissions.
// No privileged force_retry function or fabricated pgwire ErrorResponse is involved.
async fn competing_write(fixture: &Fixture) {
    bounded(fixture.observer.execute(&format!(
        "BEGIN; SET TRANSACTION PRIORITY HIGH; UPDATE {} SET value = value + 10 WHERE id = 1; COMMIT",
        fixture.table
    ))).await.unwrap();
}

async fn prepare_victim(backend: &mut CockroachTransactionBackend, table: &str) {
    bounded(backend.begin()).await.unwrap();
    bounded(backend.execute("SET TRANSACTION PRIORITY LOW"))
        .await
        .unwrap();
    bounded(backend.execute(&format!(
        "UPDATE {table} SET value = value + 1 WHERE id = 1 RETURNING value"
    )))
    .await
    .unwrap();
}

#[tokio::test]
async fn real_commit_rejection_preserves_sqlstate_and_requires_rollback() {
    let Some(fixture) = Fixture::new().await else {
        return;
    };
    bounded(
        fixture
            .observer
            .execute(&format!("INSERT INTO {} VALUES (1, 0)", fixture.table)),
    )
    .await
    .unwrap();
    let mut backend = bounded(fixture.observer.transaction_backend())
        .await
        .unwrap();
    prepare_victim(&mut backend, &fixture.table).await;
    competing_write(&fixture).await;
    let error = bounded(backend.commit()).await.unwrap_err();
    assert_eq!(error.history_snapshot().code.as_deref(), Some("40001"));
    assert!(
        error
            .relation_diagnostic()
            .unwrap()
            .message
            .contains("ROLLBACK and restart the entire transaction manually")
    );
    assert_eq!(backend.depth(), 1);
    assert!(backend.commit_failure_requires_rollback(&error));
    bounded(backend.rollback()).await.unwrap();
    assert_eq!(backend.depth(), 0);
    bounded(backend.force_close()).await.unwrap();
    assert_eq!(
        fixture.rows().await,
        vec![vec![CellValue::Integer(1), CellValue::Integer(10)]]
    );
    fixture.finish().await;
}

struct RuntimeSession {
    runtime: Runtime,
    receiver: mpsc::UnboundedReceiver<Action>,
    connection: ConnectionIdentity,
    target: ExecutionTarget,
    tab_id: Uuid,
    console_id: Uuid,
    _temp: tempfile::TempDir,
}

impl RuntimeSession {
    async fn connect(profile: ConnectionProfile) -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let (sender, receiver) = mpsc::unbounded_channel();
        let connection = ConnectionIdentity {
            profile_id: profile.id,
            generation: 1,
        };
        let target = ExecutionTarget {
            profile_id: profile.id,
            database: profile
                .database
                .clone()
                .unwrap_or_else(|| "defaultdb".into()),
            schema: Some("public".into()),
        };
        let runtime = Runtime::new(
            vec![profile],
            HashSet::new(),
            HashMap::new(),
            None,
            ProfileStore::new(temp.path().join("connections.toml")),
            Arc::new(NativeSecretStore),
            sender,
        );
        let mut session = Self {
            runtime,
            receiver,
            connection,
            target,
            tab_id: Uuid::new_v4(),
            console_id: Uuid::new_v4(),
            _temp: temp,
        };
        session.runtime.dispatch(Command::Connect {
            profile_id: connection.profile_id,
            generation: 1,
            target: session.target.clone(),
            console_id: session.console_id,
        });
        loop {
            match bounded(session.receiver.recv()).await.unwrap() {
                Action::ConnectionSucceeded { .. } => break,
                Action::ConnectionFailed { message, .. } => {
                    panic!("runtime connect failed: {message}")
                }
                _ => {}
            }
        }
        session
    }

    async fn manual_event(&mut self) -> Action {
        bounded(async {
            loop {
                let action = self
                    .receiver
                    .recv()
                    .await
                    .expect("runtime event channel closed");
                if matches!(
                    action,
                    Action::ManualStarted { .. }
                        | Action::ManualStartFailed { .. }
                        | Action::ManualQueryFinished { .. }
                        | Action::ManualQueryFailed { .. }
                        | Action::ManualQueryPageFinished { .. }
                        | Action::ManualQueryPageFailed { .. }
                        | Action::ManualCommitted { .. }
                        | Action::ManualCommitFailed { .. }
                        | Action::ManualRolledBack { .. }
                        | Action::ManualRollbackFailed { .. }
                ) {
                    return action;
                }
            }
        })
        .await
    }

    async fn execute(&mut self, sql: String) -> Result<QueryOutcome, String> {
        self.runtime.dispatch(Command::ManualExecute {
            connection: self.connection,
            target: self.target.clone(),
            tab_id: self.tab_id,
            console_id: self.console_id,
            query_generation: 1,
            transaction_generation: 1,
            sql,
        });
        loop {
            match self.manual_event().await {
                Action::ManualStarted { .. } => {}
                Action::ManualQueryFinished { outcome, .. } => return Ok(outcome),
                Action::ManualQueryFailed { message, .. } => return Err(message),
                other => panic!("unexpected runtime query event: {other:?}"),
            }
        }
    }

    async fn commit_error(&mut self) -> (bool, String) {
        self.runtime.dispatch(Command::ManualCommit {
            connection: self.connection,
            tab_id: self.tab_id,
            query_generation: 1,
            transaction_generation: 1,
        });
        match self.manual_event().await {
            Action::ManualCommitFailed {
                unknown, message, ..
            } => (unknown, message),
            other => panic!("unexpected runtime commit event: {other:?}"),
        }
    }
}

#[tokio::test]
async fn runtime_reports_unknown_and_quarantines_both_commit_disconnects_without_replay() {
    for fault in [Fault::BeforeCommit, Fault::AfterCommitSuccess] {
        let Some(fixture) = Fixture::new().await else {
            return;
        };
        let relay = Relay::start(&fixture.profile, fault).await;
        let mut session = RuntimeSession::connect(relay.profile(&fixture.profile)).await;
        let insert = format!("INSERT INTO {} VALUES (1, 7)", fixture.table);
        session.execute(insert.clone()).await.unwrap();
        let (unknown, message) = session.commit_error().await;
        assert!(unknown);
        assert!(message.contains("commit outcome is unknown"));
        assert!(message.contains("verify the result"));
        assert!(!message.contains("lazydb-relation-diagnostic"));
        // Shutdown waits for runtime worker disposal. It must not reconnect/replay to resolve COMMIT.
        bounded(session.runtime.shutdown()).await;
        relay.assert_delivery(fault, &insert);
        let expected = if fault == Fault::BeforeCommit {
            vec![]
        } else {
            vec![vec![CellValue::Integer(1), CellValue::Integer(7)]]
        };
        assert_eq!(fixture.rows().await, expected);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn runtime_manual_page_cancellation_rolls_back_writes_during_count_and_select() {
    use lazydb::model::pagination::{PageRequest, PageSize};

    for during_count in [false, true] {
        let Some(fixture) = Fixture::new().await else {
            return;
        };
        let mut session = RuntimeSession::connect(fixture.profile.clone()).await;
        session
            .execute(format!("INSERT INTO {} VALUES (1, 7)", fixture.table))
            .await
            .unwrap();
        session.runtime.dispatch(Command::ManualExecutePage {
            connection: session.connection,
            target: session.target.clone(),
            tab_id: session.tab_id,
            console_id: session.console_id,
            query_generation: 2,
            transaction_generation: 1,
            source_sql: format!("SELECT pg_sleep(30) FROM {}", fixture.table),
            dialect: lazydb::sql::SqlDialect::Postgres,
            count_sql: format!(
                "SELECT count(*) FROM {} WHERE pg_sleep(30) IS NULL",
                fixture.table
            ),
            page: if during_count {
                PageRequest::last(PageSize::Ten, 0)
            } else {
                PageRequest::first(PageSize::Ten)
            },
        });
        // Observe the actual pinned query before cancelling, avoiding a startup race.
        bounded(async {
            loop {
                let active = fixture.observer.execute(
                    "SELECT query FROM [SHOW CLUSTER QUERIES] WHERE application_name = 'lazydb'"
                ).await.unwrap();
                if active.result_sets.iter().flat_map(|set| &set.rows).any(|row| {
                    matches!(row.first(), Some(CellValue::Text(sql)) if sql.starts_with("SELECT") && sql.contains("pg_sleep") && sql.contains(&fixture.table) && !sql.contains("SHOW CLUSTER QUERIES"))
                }) { break; }
                tokio::task::yield_now().await;
            }
        }).await;
        session.runtime.dispatch(Command::CancelManual {
            connection: session.connection,
            tab_id: session.tab_id,
            query_generation: 2,
            transaction_generation: 1,
        });
        assert!(
            matches!(session.manual_event().await, Action::ManualQueryPageFailed { message, .. } if message == "cancelled")
        );
        bounded(session.runtime.shutdown()).await;
        assert!(
            fixture.rows().await.is_empty(),
            "cancelling a manual page must discard pending writes"
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn runtime_rejected_commit_allows_only_rollback_and_never_replays_sql() {
    let Some(fixture) = Fixture::new().await else {
        return;
    };
    bounded(
        fixture
            .observer
            .execute(&format!("INSERT INTO {} VALUES (1, 0)", fixture.table)),
    )
    .await
    .unwrap();
    let relay = Relay::start(&fixture.profile, Fault::PassThrough).await;
    let mut session = RuntimeSession::connect(relay.profile(&fixture.profile)).await;
    session
        .execute("SET TRANSACTION PRIORITY LOW".into())
        .await
        .unwrap();
    session
        .execute(format!(
            "UPDATE {} SET value = value + 1 WHERE id = 1 RETURNING value",
            fixture.table
        ))
        .await
        .unwrap();
    competing_write(&fixture).await;
    let (unknown, message) = session.commit_error().await;
    assert!(!unknown, "an explicit server rejection has a known outcome");
    assert!(message.contains("40001"));
    assert!(message.contains("ROLLBACK and restart"));
    let before = relay.evidence.lock().unwrap().sql.clone();
    let blocked = session
        .execute(format!("INSERT INTO {} VALUES (2, 99)", fixture.table))
        .await
        .unwrap_err();
    assert!(blocked.contains("ROLLBACK is required"));
    let (unknown, message) = session.commit_error().await;
    assert!(!unknown);
    assert!(message.contains("ROLLBACK is required"));
    assert_eq!(
        relay.evidence.lock().unwrap().sql,
        before,
        "rollback-only requests must not reach the server"
    );
    session.runtime.dispatch(Command::ManualRollback {
        connection: session.connection,
        tab_id: session.tab_id,
        query_generation: 1,
        transaction_generation: 1,
    });
    assert!(matches!(
        session.manual_event().await,
        Action::ManualRolledBack { .. }
    ));
    bounded(session.runtime.shutdown()).await;
    let sql = relay.evidence.lock().unwrap().sql.clone();
    assert_eq!(sql.iter().filter(|sql| is_commit(sql)).count(), 1);
    assert_eq!(
        sql.iter().filter(|sql| sql.starts_with("UPDATE ")).count(),
        1
    );
    assert_eq!(
        sql.iter()
            .filter(|sql| sql.eq_ignore_ascii_case("ROLLBACK"))
            .count(),
        1
    );
    assert_eq!(
        fixture.rows().await,
        vec![vec![CellValue::Integer(1), CellValue::Integer(10)]]
    );
    fixture.finish().await;
}
