#![cfg(unix)]

#[path = "fixtures/ssh/test_support.rs"]
mod support;

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use lazydb::{
    connections::DatabaseConnector,
    db::{DatabaseConnection, ErrorCategory, transaction::TransactionBackend, value::CellValue},
    profile::{ConnectionProfile, PgConnectionOptions, SshOptions, SslMode, import_connection_url},
    ssh::TunnelManager,
};
use secrecy::SecretString;
use sqlx::{
    Connection, Row,
    mysql::{MySqlConnectOptions, MySqlConnection, MySqlSslMode},
    postgres::{PgConnectOptions, PgConnection, PgSslMode},
};
use support::{
    FixtureManifest, MYSQL_PASSWORD_ENV, POSTGRES_PASSWORD_ENV, SshForwarder,
    assert_tls_verification_error, fixture, fixture_tempdir, kill_managed_ssh_tunnels,
    required_env,
};

fn postgres_ssh_profile(fixture: &FixtureManifest, ca_file: &Path) -> ConnectionProfile {
    let url = format!(
        "postgresql://{}@{}:{}/{}",
        fixture.postgres.user,
        fixture.postgres.host,
        fixture.postgres.port,
        fixture.postgres.database
    );
    let mut profile = import_connection_url(&url, Some("postgres-ssh-connector"))
        .expect("import PostgreSQL fixture URL")
        .profile;
    profile.ssl_mode = SslMode::VerifyFull;
    profile.pg_options = Some(PgConnectionOptions {
        ssl_root_cert: Some(ca_file.to_path_buf()),
        ssl_client_cert: Some(fixture.postgres.client_cert.clone()),
        ssl_client_key: Some(fixture.postgres.client_key.clone()),
        options: None,
    });
    profile.ssh = Some(SshOptions {
        host: fixture.ssh.host.clone(),
        port: Some(fixture.ssh.port),
        user: Some(fixture.ssh.user.clone()),
        identity_file: Some(fixture.ssh.client_key.clone()),
        config_file: Some(fixture.ssh.config_file.clone()),
        known_hosts_file: Some(fixture.ssh.known_hosts_file.clone()),
        proxy_jump: None,
    });
    profile
}

async fn assert_no_active_tunnels(connector: &DatabaseConnector) {
    for _ in 0..100 {
        if connector.active_tunnel_count() == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!(
        "connector retained {} SSH tunnel(s) after the final lease was dropped",
        connector.active_tunnel_count()
    );
}

fn postgres_options(
    fixture: &FixtureManifest,
    socket_directory: &Path,
    host: &str,
    ca_file: &Path,
) -> PgConnectOptions {
    PgConnectOptions::new()
        .host(host)
        .port(fixture.postgres.port)
        .socket(socket_directory)
        .username(&fixture.postgres.user)
        .password(&required_env(POSTGRES_PASSWORD_ENV))
        .database(&fixture.postgres.database)
        .ssl_mode(PgSslMode::VerifyFull)
        .ssl_root_cert(ca_file)
        .ssl_client_cert(&fixture.postgres.client_cert)
        .ssl_client_key(&fixture.postgres.client_key)
}

fn mysql_options(
    fixture: &FixtureManifest,
    socket_path: &Path,
    host: &str,
    ca_file: &Path,
) -> MySqlConnectOptions {
    MySqlConnectOptions::new()
        .host(host)
        .port(fixture.mysql.port)
        .socket(socket_path)
        .username(&fixture.mysql.user)
        .password(&required_env(MYSQL_PASSWORD_ENV))
        .database(&fixture.mysql.database)
        .ssl_mode(MySqlSslMode::VerifyIdentity)
        .ssl_ca(ca_file)
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn postgres_uds_forward_preserves_original_hostname_and_strict_tls_in_pool() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory
        .path()
        .join(format!(".s.PGSQL.{}", fixture.postgres.port));
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.postgres.host,
        fixture.postgres.port,
    )
    .await;
    let options = postgres_options(
        &fixture,
        socket_directory.path(),
        &fixture.postgres.host,
        &fixture.postgres.ca_file,
    );
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("connect PostgreSQL pool through SQLx UDS transport");
    let mut first = pool.acquire().await.expect("first PostgreSQL connection");
    let mut second = pool.acquire().await.expect("second PostgreSQL connection");

    let first_identity = sqlx::query(
        "SELECT s.ssl AS ssl, s.version AS tls_version, s.client_dn AS client_dn, \
                pg_backend_pid() AS pid, current_database()::text AS database_name, \
                current_user::text AS database_user, inet_client_addr()::text AS client_addr \
         FROM pg_stat_ssl s WHERE s.pid = pg_backend_pid()",
    )
    .fetch_one(&mut *first)
    .await
    .expect("inspect PostgreSQL TLS session");
    let second_identity = sqlx::query(
        "SELECT s.ssl AS ssl, s.version AS tls_version, pg_backend_pid() AS pid, \
                inet_client_addr()::text AS client_addr \
         FROM pg_stat_ssl s WHERE s.pid = pg_backend_pid()",
    )
    .fetch_one(&mut *second)
    .await
    .expect("inspect second PostgreSQL TLS session");

    assert!(
        first_identity.get::<bool, _>("ssl"),
        "PostgreSQL pool connection was not encrypted"
    );
    assert!(
        second_identity.get::<bool, _>("ssl"),
        "second PostgreSQL pool connection was not encrypted"
    );
    assert_ne!(
        first_identity.get::<i32, _>("pid"),
        second_identity.get::<i32, _>("pid")
    );
    assert_eq!(
        first_identity.get::<String, _>("database_name"),
        fixture.postgres.database
    );
    assert_eq!(
        first_identity.get::<String, _>("database_user"),
        fixture.postgres.user
    );
    assert!(
        first_identity
            .get::<String, _>("tls_version")
            .starts_with("TLSv1.")
    );
    assert!(
        first_identity
            .get::<Option<String>, _>("client_dn")
            .is_some_and(|dn| dn.contains("lazydb")),
        "PostgreSQL did not report the verified client certificate"
    );
    for row in [&first_identity, &second_identity] {
        let client_addr = row
            .get::<Option<String>, _>("client_addr")
            .expect("PostgreSQL peer address");
        assert_ne!(client_addr, "127.0.0.1");
        assert_ne!(client_addr, "::1");
    }

    drop(first);
    drop(second);
    pool.close().await;
    forwarder.assert_ready_marker_order();
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn postgres_uds_forward_rejects_wrong_ca() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory
        .path()
        .join(format!(".s.PGSQL.{}", fixture.postgres.port));
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.postgres.host,
        fixture.postgres.port,
    )
    .await;
    let options = postgres_options(
        &fixture,
        socket_directory.path(),
        &fixture.postgres.host,
        &fixture.postgres.wrong_ca_file,
    );
    let error = PgConnection::connect_with(&options)
        .await
        .expect_err("PostgreSQL accepted an unrelated CA");
    assert_tls_verification_error(error, "UnknownIssuer", "PostgreSQL wrong CA");
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn postgres_uds_forward_rejects_wrong_hostname() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory
        .path()
        .join(format!(".s.PGSQL.{}", fixture.postgres.port));
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.postgres.host,
        fixture.postgres.port,
    )
    .await;
    let options = postgres_options(
        &fixture,
        socket_directory.path(),
        "wrong-postgres.internal",
        &fixture.postgres.ca_file,
    );
    let error = PgConnection::connect_with(&options)
        .await
        .expect_err("PostgreSQL accepted the wrong TLS hostname");
    assert_tls_verification_error(error, "NotValidForName", "PostgreSQL wrong hostname");
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn postgres_uds_forward_rejects_a_server_without_tls() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory
        .path()
        .join(format!(".s.PGSQL.{}", fixture.postgres.port));
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.postgres.host,
        fixture.postgres.port,
    )
    .await;
    let options = postgres_options(
        &fixture,
        socket_directory.path(),
        &fixture.postgres.host,
        &fixture.postgres.ca_file,
    )
    .ssl_mode(PgSslMode::Disable);
    let error = PgConnection::connect_with(&options)
        .await
        .expect_err("PostgreSQL accepted a non-TLS connection");
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("no encryption"),
        "unexpected non-TLS PostgreSQL rejection: {error:?}"
    );
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mysql_uds_forward_preserves_original_hostname_and_strict_tls() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory.path().join("mysql.sock");
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.mysql.host,
        fixture.mysql.port,
    )
    .await;
    let options = mysql_options(
        &fixture,
        &socket_path,
        &fixture.mysql.host,
        &fixture.mysql.ca_file,
    );
    let mut connection = MySqlConnection::connect_with(&options)
        .await
        .expect("connect MySQL through SQLx UDS transport");
    let tls = sqlx::query("SHOW SESSION STATUS LIKE 'Ssl_version'")
        .fetch_one(&mut connection)
        .await
        .expect("inspect MySQL TLS session");
    let identity =
        sqlx::query("SELECT DATABASE() AS database_name, CURRENT_USER() AS database_user")
            .fetch_one(&mut connection)
            .await
            .expect("inspect MySQL session identity");

    let tls_version: String = tls.get(1);
    assert!(tls_version.starts_with("TLSv1."), "{tls_version}");
    assert_eq!(
        identity.get::<Option<String>, _>("database_name"),
        Some(fixture.mysql.database.clone())
    );
    assert!(
        identity
            .get::<String, _>("database_user")
            .starts_with(&fixture.mysql.user)
    );
    connection.close().await.expect("close MySQL connection");
    forwarder.assert_ready_marker_order();
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mysql_uds_forward_rejects_wrong_ca() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory.path().join("mysql.sock");
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.mysql.host,
        fixture.mysql.port,
    )
    .await;
    let options = mysql_options(
        &fixture,
        &socket_path,
        &fixture.mysql.host,
        &fixture.mysql.wrong_ca_file,
    );
    let error = MySqlConnection::connect_with(&options)
        .await
        .expect_err("MySQL accepted an unrelated CA");
    assert_tls_verification_error(error, "UnknownIssuer", "MySQL wrong CA");
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mysql_uds_forward_rejects_wrong_hostname() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory.path().join("mysql.sock");
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.mysql.host,
        fixture.mysql.port,
    )
    .await;
    let options = mysql_options(
        &fixture,
        &socket_path,
        "wrong-mysql.internal",
        &fixture.mysql.ca_file,
    );
    let error = MySqlConnection::connect_with(&options)
        .await
        .expect_err("MySQL accepted the wrong TLS hostname");
    assert_tls_verification_error(error, "NotValidForName", "MySQL wrong hostname");
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mysql_uds_forward_rejects_a_server_without_tls() {
    let fixture = fixture();
    let socket_directory = fixture_tempdir();
    let socket_path = socket_directory.path().join("mysql.sock");
    let forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.mysql.host,
        fixture.mysql.port,
    )
    .await;
    let options = mysql_options(
        &fixture,
        &socket_path,
        &fixture.mysql.host,
        &fixture.mysql.ca_file,
    )
    .ssl_mode(MySqlSslMode::Disabled);
    let error = MySqlConnection::connect_with(&options)
        .await
        .expect_err("MySQL accepted a non-TLS connection");
    let sqlx::Error::Database(database) = &error else {
        panic!("unexpected non-TLS MySQL rejection: {error:?}");
    };
    assert_eq!(database.code().as_deref(), Some("28000"));
    let mysql = database
        .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
        .expect("MySQL server error");
    assert_eq!(mysql.number(), 1045);
    forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn database_connector_reuses_ssh_pg_tunnel_and_preserves_strict_tls() {
    let fixture = fixture();
    let profile = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file);
    let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
    let connector = DatabaseConnector::new();
    assert_eq!(connector.active_tunnel_count(), 0);

    let first = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect first PostgreSQL connection through DatabaseConnector");
    assert_eq!(connector.active_tunnel_count(), 1);
    let second = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect second PostgreSQL connection through a shared SSH tunnel");
    assert_eq!(connector.active_tunnel_count(), 1);

    let first_identity = first
        .execute(
            "SELECT s.ssl, s.version, s.client_dn, inet_client_addr()::text AS client_addr \
             FROM pg_stat_ssl s WHERE s.pid = pg_backend_pid()",
        )
        .await
        .expect("query first routed PostgreSQL identity");
    let second_identity = second
        .execute(
            "SELECT s.ssl, s.version, s.client_dn, inet_client_addr()::text AS client_addr \
             FROM pg_stat_ssl s WHERE s.pid = pg_backend_pid()",
        )
        .await
        .expect("query second routed PostgreSQL identity");
    for outcome in [&first_identity, &second_identity] {
        let row = &outcome
            .result_sets
            .last()
            .expect("identity result set")
            .rows[0];
        assert_eq!(row[0], lazydb::db::value::CellValue::Boolean(true));
        let tls_version = match &row[1] {
            lazydb::db::value::CellValue::Text(version) => version,
            value => panic!("unexpected TLS version cell: {value:?}"),
        };
        assert!(tls_version.starts_with("TLSv1."), "{tls_version}");
        let client_addr = match &row[3] {
            lazydb::db::value::CellValue::Text(address) => address,
            value => panic!("unexpected client address cell: {value:?}"),
        };
        assert_ne!(client_addr, "127.0.0.1");
        assert_ne!(client_addr, "::1");
    }
    let client_dn = match &first_identity.result_sets.last().unwrap().rows[0][2] {
        lazydb::db::value::CellValue::Text(dn) => dn,
        value => panic!("unexpected client certificate subject cell: {value:?}"),
    };
    assert!(client_dn.contains("lazydb"), "{client_dn}");

    first.close().await;
    assert_eq!(
        connector.active_tunnel_count(),
        1,
        "closing one pooled connection released the tunnel still used by the other"
    );
    second.close().await;
    assert_no_active_tunnels(&connector).await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn failed_sqlx_route_is_not_rewired_to_a_new_tunnel() {
    let fixture = fixture();
    let profile = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file);
    let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
    let connector = DatabaseConnector::new();

    let stale = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect PostgreSQL before killing its managed tunnel");
    stale
        .execute("SELECT 1")
        .await
        .expect("query before killing the managed tunnel");

    kill_managed_ssh_tunnels(&fixture.ssh.client_key);
    assert_no_active_tunnels(&connector).await;
    let stale_error = tokio::time::timeout(Duration::from_secs(5), stale.execute("SELECT 1"))
        .await
        .expect("old SQLx connection hung after tunnel exit")
        .expect_err("old SQLx connection reused a tunnel that had exited");
    assert_ne!(stale_error.category, ErrorCategory::Cancelled);

    let replacement = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect a replacement PostgreSQL session through a new tunnel");
    assert_eq!(connector.active_tunnel_count(), 1);
    replacement
        .execute("SELECT 1")
        .await
        .expect("replacement PostgreSQL session did not use its new tunnel");
    stale
        .execute("SELECT 1")
        .await
        .expect_err("old PostgreSQL session was silently moved to the replacement tunnel");

    stale.close().await;
    replacement.close().await;
    assert_no_active_tunnels(&connector).await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn database_connector_preserves_required_postgres_tls_with_unix_forwarding() {
    let fixture = fixture();
    let mut profile = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file);
    profile.ssl_mode = SslMode::Require;
    let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
    let connector = DatabaseConnector::new();

    let connection = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect PostgreSQL with required TLS through the managed Unix socket route");
    let identity = connection
        .execute(
            "SELECT s.ssl, s.version, inet_client_addr()::text AS client_addr \
             FROM pg_stat_ssl s WHERE s.pid = pg_backend_pid()",
        )
        .await
        .expect("inspect required PostgreSQL TLS session");
    let row = &identity
        .result_sets
        .last()
        .expect("identity result set")
        .rows[0];
    assert_eq!(row[0], lazydb::db::value::CellValue::Boolean(true));
    let tls_version = match &row[1] {
        lazydb::db::value::CellValue::Text(version) => version,
        value => panic!("unexpected TLS version cell: {value:?}"),
    };
    assert!(tls_version.starts_with("TLSv1."), "{tls_version}");
    let client_addr = match &row[2] {
        lazydb::db::value::CellValue::Text(address) => address,
        value => panic!("unexpected client address cell: {value:?}"),
    };
    assert_ne!(client_addr, "127.0.0.1");
    assert_ne!(client_addr, "::1");

    connection.close().await;
    assert_no_active_tunnels(&connector).await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn database_connector_rejects_wrong_ca_and_releases_the_tunnel() {
    let fixture = fixture();
    let profile = postgres_ssh_profile(&fixture, &fixture.postgres.wrong_ca_file);
    let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
    let connector = DatabaseConnector::new();

    let error = connector
        .connect(&profile, Some(&password))
        .await
        .expect_err("DatabaseConnector accepted an unrelated PostgreSQL CA");
    assert_eq!(error.category, ErrorCategory::Network);
    assert!(
        error.message.contains("UnknownIssuer"),
        "unexpected strict TLS error: {error:?}"
    );
    assert_no_active_tunnels(&connector).await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn concurrent_tunnel_waiters_share_one_process_and_cancel_independently() {
    let fixture = fixture();
    let profile = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file);
    let manager = TunnelManager::default();
    let mut waiters = Vec::new();
    for _ in 0..20 {
        let manager = manager.clone();
        let profile = profile.clone();
        waiters.push(tokio::spawn(async move { manager.acquire(&profile).await }));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while manager.active_count() == 0 {
        assert!(
            Instant::now() < deadline,
            "SSH waiters never started a tunnel"
        );
        tokio::task::yield_now().await;
    }
    for waiter in waiters.drain(..5) {
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
    }
    let mut leases = Vec::new();
    for waiter in waiters {
        leases.push(waiter.await.unwrap().expect("shared SSH startup failed"));
    }
    assert_eq!(leases.len(), 15);
    assert_eq!(manager.active_count(), 1);
    let processes = support::kill_managed_ssh_tunnels(&fixture.ssh.client_key);
    assert_eq!(
        processes.len(),
        1,
        "same-key waiters started multiple ssh processes"
    );
    for lease in &leases {
        tokio::time::timeout(Duration::from_secs(5), lease.failure())
            .await
            .expect("tunnel exit did not wake every lease");
        assert!(lease.ensure_alive().is_err());
    }
    drop(leases);
    manager.shutdown().await;
    assert_eq!(manager.active_count(), 0);
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py and ssh-agent"]
async fn ssh_config_can_select_a_key_held_only_by_an_agent() {
    let fixture = fixture();
    let directory = fixture_tempdir();
    let socket = directory.path().join("agent.sock");
    let mut agent = tokio::process::Command::new("ssh-agent")
        .args(["-D", "-a"])
        .arg(&socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("start an isolated SSH agent");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        assert!(
            agent.try_wait().unwrap().is_none(),
            "SSH agent exited before listening"
        );
        assert!(
            Instant::now() < deadline,
            "SSH agent did not create its socket"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let status = tokio::process::Command::new("ssh-add")
        .arg("-q")
        .arg(&fixture.ssh.client_key)
        .env("SSH_AUTH_SOCK", &socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(
        status.success(),
        "could not load the disposable fixture key"
    );
    let config = directory.path().join("config with spaces");
    std::fs::write(
        &config,
        format!(
            "Host *\n  IdentitiesOnly yes\n  IdentityFile \"{}\"\n  IdentityAgent \"{}\"\n",
            fixture.ssh.client_key.with_extension("pub").display(),
            socket.display()
        ),
    )
    .unwrap();
    let mut profile = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file);
    let ssh = profile.ssh.as_mut().unwrap();
    ssh.identity_file = None;
    ssh.config_file = Some(config);
    let manager = TunnelManager::default();
    let lease = manager
        .acquire(&profile)
        .await
        .expect("authenticate using the agent-only key");
    lease.ensure_alive().unwrap();
    manager.shutdown().await;
    agent.kill().await.unwrap();
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn postgres_full_pool_cancellation_keeps_the_control_connection_routed() {
    let fixture = fixture();
    let profile = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file);
    let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
    let connector = DatabaseConnector::new();
    let database = Arc::new(connector.connect(&profile, Some(&password)).await.unwrap());
    let observer = connector.connect(&profile, Some(&password)).await.unwrap();
    let query = format!(
        "SELECT pg_sleep(30) /* ssh_cancel_{} */",
        uuid::Uuid::new_v4().simple()
    );
    let mut queries = Vec::new();
    for _ in 0..6 {
        let database = Arc::clone(&database);
        let query = query.clone();
        queries.push(tokio::spawn(async move { database.execute(&query).await }));
    }
    let count_sql = format!(
        "SELECT count(*) FROM pg_stat_activity WHERE state = 'active' AND query = '{}'",
        query
    );
    wait_for_count(&observer, &count_sql, 6).await;
    for query in queries {
        query.abort();
        assert!(query.await.unwrap_err().is_cancelled());
    }
    wait_for_count(&observer, &count_sql, 0).await;
    assert_eq!(connector.active_tunnel_count(), 1);
    assert_eq!(
        database
            .execute("SELECT 42::bigint")
            .await
            .unwrap()
            .result_sets[0]
            .rows[0][0],
        CellValue::Integer(42)
    );
    observer.close().await;
    Arc::try_unwrap(database).unwrap().close().await;
    assert_no_active_tunnels(&connector).await;
    connector.shutdown().await;
}

async fn wait_for_count(database: &DatabaseConnection, sql: &str, expected: i64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let outcome = database.execute(sql).await.unwrap();
        if outcome.result_sets[0].rows[0][0] == CellValue::Integer(expected) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "server activity never reached {expected}: {outcome:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn postgres_transaction_retains_the_last_lease_until_rollback_and_close() {
    let fixture = fixture();
    let profile = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file);
    let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
    let connector = DatabaseConnector::new();
    let connection = connector.connect(&profile, Some(&password)).await.unwrap();
    let mut transaction = match &connection {
        DatabaseConnection::Postgres(adapter) => adapter.transaction_backend().await.unwrap(),
        _ => panic!("expected PostgreSQL"),
    };
    transaction.begin().await.unwrap();
    transaction
        .execute("CREATE TEMP TABLE lazydb_ssh_transaction (value integer)")
        .await
        .unwrap();
    transaction
        .execute("INSERT INTO lazydb_ssh_transaction VALUES (7)")
        .await
        .unwrap();
    drop(connection);
    assert_eq!(connector.active_tunnel_count(), 1);
    assert_eq!(
        transaction
            .execute("SELECT value::bigint FROM lazydb_ssh_transaction")
            .await
            .unwrap()
            .result_sets[0]
            .rows[0][0],
        CellValue::Integer(7)
    );
    transaction.rollback().await.unwrap();
    let closing = transaction.force_close();
    assert_eq!(connector.active_tunnel_count(), 1);
    closing.await.unwrap();
    assert_no_active_tunnels(&connector).await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn postgres_database_targets_reuse_the_same_tunnel() {
    let fixture = fixture();
    let mut profile = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file);
    profile.catalog_scope.databases = lazydb::profile::CatalogSelection::All;
    let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
    let connector = DatabaseConnector::new();
    let connection = connector.connect(&profile, Some(&password)).await.unwrap();
    let target = lazydb::model::execution_target::ExecutionTarget {
        profile_id: profile.id,
        database: "postgres".into(),
        schema: Some("public".into()),
    };
    let second = connector
        .connect_target(&profile, Some(&password), &target)
        .await
        .unwrap();
    assert_eq!(second.probe().await.unwrap().database, "postgres");
    assert_eq!(connector.active_tunnel_count(), 1);
    connection.close().await;
    second.execute("SELECT 1").await.unwrap();
    second.close().await;
    assert_no_active_tunnels(&connector).await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn mysql_managed_tunnel_preserves_auth_tls_and_transaction_semantics() {
    let fixture = fixture();
    let mut profile = import_connection_url(
        &format!(
            "mysql://{}@{}:{}/{}",
            fixture.mysql.user, fixture.mysql.host, fixture.mysql.port, fixture.mysql.database
        ),
        Some("mysql-managed-ssh"),
    )
    .unwrap()
    .profile;
    profile.ssh = postgres_ssh_profile(&fixture, &fixture.postgres.ca_file).ssh;
    profile.ssl_mode = SslMode::Require;
    let password = SecretString::from(required_env(MYSQL_PASSWORD_ENV));
    let connector = DatabaseConnector::new();
    let connection = connector.connect(&profile, Some(&password)).await.unwrap();
    let outcome = connection
        .execute("SHOW SESSION STATUS LIKE 'Ssl_cipher'")
        .await
        .unwrap();
    assert!(
        matches!(&outcome.result_sets[0].rows[0][1], CellValue::Text(cipher) if !cipher.is_empty())
    );
    let table = format!("lazydb_ssh_{}", uuid::Uuid::new_v4().simple());
    connection
        .execute(&format!("CREATE TABLE {table} (value integer)"))
        .await
        .unwrap();
    connection
        .execute(&format!(
            "START TRANSACTION; INSERT INTO {table} VALUES (7); ROLLBACK"
        ))
        .await
        .unwrap();
    let count = connection
        .execute(&format!("SELECT count(*) FROM {table}"))
        .await
        .unwrap();
    assert!(matches!(
        count.result_sets[0].rows[0][0],
        CellValue::Integer(0) | CellValue::Unsigned(0)
    ));
    connection
        .execute(&format!("DROP TABLE {table}"))
        .await
        .unwrap();
    connection.close().await;
    assert_no_active_tunnels(&connector).await;
    connector.shutdown().await;
}
