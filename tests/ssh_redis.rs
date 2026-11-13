#![cfg(unix)]

#[path = "fixtures/ssh/test_support.rs"]
mod support;

use std::{
    env,
    io::ErrorKind,
    net::{SocketAddr, TcpListener},
    time::{Duration, Instant},
};

use lazydb::{
    connections::DatabaseConnector,
    db::{DatabaseConnection, ErrorCategory},
    profile::{ConnectionProfile, DatabaseKind, SshOptions, SslMode, import_connection_url},
};
use secrecy::{ExposeSecret, SecretString};
use support::{
    FixtureManifest, REDIS_PASSWORD_ENV, fixture, kill_managed_ssh_tunnels, required_env,
};
use uuid::Uuid;

fn ssh_options(fixture: &FixtureManifest) -> SshOptions {
    SshOptions {
        host: fixture.ssh.host.clone(),
        port: Some(fixture.ssh.port),
        user: Some(fixture.ssh.user.clone()),
        identity_file: Some(fixture.ssh.client_key.clone()),
        config_file: Some(fixture.ssh.config_file.clone()),
        known_hosts_file: Some(fixture.ssh.known_hosts_file.clone()),
        proxy_jump: None,
    }
}

fn routed_profile(
    fixture: &FixtureManifest,
    url: &str,
    name: &str,
) -> (ConnectionProfile, SecretString) {
    let imported = import_connection_url(url, Some(name)).unwrap();
    let mut profile = imported.profile;
    profile.ssh = Some(ssh_options(fixture));
    (
        profile,
        SecretString::from(required_env(REDIS_PASSWORD_ENV)),
    )
}

fn routed_forward_address() -> SocketAddr {
    required_env("LAZYDB_TEST_SSH_REDIS_ROUTED_ADDR")
        .parse()
        .expect("valid routed Redis address")
}

fn proxy_jump_ssh_options(fixture: &FixtureManifest) -> SshOptions {
    SshOptions {
        host: fixture.jump.final_host.clone(),
        port: Some(fixture.jump.final_port),
        user: Some(fixture.ssh.user.clone()),
        identity_file: Some(fixture.ssh.client_key.clone()),
        config_file: Some(fixture.ssh.config_file.clone()),
        known_hosts_file: Some(fixture.ssh.known_hosts_file.clone()),
        proxy_jump: Some(fixture.jump.proxy_alias.clone()),
    }
}

async fn seed_connection(
    database: u16,
    password: &SecretString,
) -> redis::aio::MultiplexedConnection {
    let address = routed_forward_address();
    let mut connection = redis::Client::open((address.ip().to_string(), address.port()))
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .expect("connect to the fixture's pre-existing routed Redis endpoint");
    redis::cmd("AUTH")
        .arg(password.expose_secret())
        .query_async::<()>(&mut connection)
        .await
        .expect("authenticate fixture seed connection");
    redis::cmd("SELECT")
        .arg(database)
        .query_async::<()>(&mut connection)
        .await
        .expect("select fixture database");
    connection
}

async fn scan_all(adapter: &lazydb::db::redis::RedisAdapter, pattern: &str) -> Vec<Vec<u8>> {
    let mut cursor = 0;
    let mut keys = Vec::new();
    loop {
        let (next, mut page) = adapter
            .scan_keys(cursor, pattern.as_bytes(), 100)
            .await
            .unwrap();
        keys.append(&mut page);
        if next == 0 {
            return keys;
        }
        cursor = next;
    }
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn routed_valkey_preserves_auth_select_scan_and_delete() {
    let fixture = fixture();
    let url = required_env("LAZYDB_TEST_SSH_REDIS_URL").replacen("redis://", "valkey://", 1);
    let database: u16 = required_env("LAZYDB_TEST_SSH_REDIS_DB2").parse().unwrap();
    let (mut profile, password) = routed_profile(&fixture, &url, "ssh-valkey-routed");
    profile.database = Some(database.to_string());
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect routed Valkey through managed SSH");

    assert_eq!(profile.kind, DatabaseKind::Valkey);
    assert_eq!(connection.kind(), DatabaseKind::Valkey);
    assert_eq!(connector.active_tunnel_count(), 1);
    let probe = connection.probe().await.unwrap();
    assert_eq!(probe.kind, DatabaseKind::Valkey);
    assert_eq!(probe.database, database.to_string());
    assert!(
        probe.version.starts_with("8.1."),
        "unexpected fixture version {}",
        probe.version
    );

    let adapter = match &connection {
        DatabaseConnection::Redis(adapter) => adapter,
        _ => panic!("Valkey must use the shared Redis-compatible adapter"),
    };
    assert_eq!(adapter.database(), u32::from(database));

    let prefix = format!("lazydb:ssh-redis:{}", Uuid::new_v4());
    let string_key = format!("{prefix}:string");
    let hash_key = format!("{prefix}:hash");
    let pending_key = format!("{prefix}:pending");
    let keys = [&string_key, &hash_key, &pending_key];
    let mut seed = seed_connection(database, &password).await;
    for key in keys {
        redis::cmd("SET")
            .arg(key)
            .arg("routed")
            .query_async::<()>(&mut seed)
            .await
            .unwrap();
    }

    let pattern = format!("{prefix}:*");
    let mut found = scan_all(adapter, &pattern).await;
    found.sort();
    let mut expected = keys
        .iter()
        .map(|key| key.as_bytes().to_vec())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(found, expected);

    assert_eq!(adapter.delete_key(pending_key.as_bytes()).await.unwrap(), 1);
    let remaining = scan_all(adapter, &pattern).await;
    assert!(!remaining.iter().any(|key| key == pending_key.as_bytes()));
    adapter.ping().await.unwrap();

    for key in keys {
        redis::cmd("DEL")
            .arg(key)
            .query_async::<()>(&mut seed)
            .await
            .unwrap();
    }
    connection.close().await;
    connector.shutdown().await;
    assert_eq!(connector.active_tunnel_count(), 0);
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn routed_redis_wrong_password_fails_after_reaching_the_tunneled_server() {
    let fixture = fixture();
    let url = required_env("LAZYDB_TEST_SSH_REDIS_URL");
    let (profile, password) = routed_profile(&fixture, &url, "ssh-redis-wrong-password");
    let wrong_password = SecretString::from(format!("{}-wrong", password.expose_secret()));
    let connector = DatabaseConnector::new();
    let error = connector
        .connect(&profile, Some(&wrong_password))
        .await
        .expect_err("routed Redis accepted a wrong password");
    assert_eq!(error.category, ErrorCategory::Authentication);
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py and SSL_CERT_FILE"]
async fn routed_redis_tls_keeps_the_logical_hostname() {
    let fixture = fixture();
    let ca_file = required_env("LAZYDB_TEST_SSH_REDIS_TLS_CA_FILE");
    assert_eq!(
        env::var("SSL_CERT_FILE").ok().as_deref(),
        Some(ca_file.as_str()),
        "run this test with SSL_CERT_FILE set to LAZYDB_TEST_SSH_REDIS_TLS_CA_FILE"
    );
    let url = required_env("LAZYDB_TEST_SSH_REDIS_TLS_URL");
    let (mut profile, password) = routed_profile(&fixture, &url, "ssh-redis-tls");
    profile.ssl_mode = SslMode::VerifyFull;
    assert_eq!(profile.ssl_mode, SslMode::VerifyFull);
    profile.host = Some("redis.internal".into());
    let connector = DatabaseConnector::new();
    let connection = tokio::time::timeout(
        Duration::from_secs(30),
        connector.connect(&profile, Some(&password)),
    )
    .await
    .expect("routed TLS Redis connection timed out")
    .expect("connect routed TLS Redis through managed SSH");
    match &connection {
        DatabaseConnection::Redis(adapter) => adapter.ping().await.unwrap(),
        _ => panic!("TLS Redis must use the shared Redis-compatible adapter"),
    }
    connection.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn connector_shutdown_invalidates_the_routed_redis_transport() {
    let fixture = fixture();
    let url = required_env("LAZYDB_TEST_SSH_REDIS_URL");
    let (profile, password) = routed_profile(&fixture, &url, "ssh-redis-shutdown");
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect routed Redis");
    let adapter = match &connection {
        DatabaseConnection::Redis(adapter) => adapter,
        _ => panic!("Redis must use the shared Redis-compatible adapter"),
    };
    adapter.ping().await.unwrap();

    connector.shutdown().await;
    assert_eq!(connector.active_tunnel_count(), 0);
    let error = tokio::time::timeout(Duration::from_secs(2), adapter.ping())
        .await
        .expect("dead tunnel command did not fail promptly")
        .expect_err("routed Redis reused a transport after its tunnel shut down");
    assert_eq!(error.category, ErrorCategory::Network);
    assert_eq!(error.code.as_deref(), Some("ssh_tunnel_unavailable"));
    connection.close().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn routed_redis_uses_proxy_jump_through_the_managed_tunnel() {
    let fixture = fixture();
    let url = required_env("LAZYDB_TEST_SSH_REDIS_URL");
    let (mut profile, password) = routed_profile(&fixture, &url, "ssh-redis-proxy-jump");
    profile.ssh = Some(proxy_jump_ssh_options(&fixture));
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect routed Redis through a managed ProxyJump");
    let adapter = match &connection {
        DatabaseConnection::Redis(adapter) => adapter,
        _ => panic!("Redis must use the shared Redis-compatible adapter"),
    };

    adapter.ping().await.unwrap();
    assert_eq!(connector.active_tunnel_count(), 1);

    connection.close().await;
    connector.shutdown().await;
    assert_eq!(connector.active_tunnel_count(), 0);
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn crashed_redis_tunnel_fails_closed_before_reusing_its_loopback_port() {
    let fixture = fixture();
    let url = required_env("LAZYDB_TEST_SSH_REDIS_URL");
    let (profile, password) = routed_profile(&fixture, &url, "ssh-redis-tunnel-crash");
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, Some(&password))
        .await
        .expect("connect routed Redis before tunnel crash");
    let adapter = match &connection {
        DatabaseConnection::Redis(adapter) => adapter,
        _ => panic!("Redis must use the shared Redis-compatible adapter"),
    };
    let released = kill_managed_ssh_tunnels(&fixture.ssh.client_key);
    let port = released
        .iter()
        .find_map(|tunnel| tunnel.local_port)
        .expect("managed Redis tunnel did not expose a loopback port");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match adapter.ping().await {
            Err(error) if error.code.as_deref() == Some("ssh_tunnel_unavailable") => {
                assert_eq!(error.category, ErrorCategory::Network);
                break;
            }
            Err(error) => assert!(
                Instant::now() < deadline,
                "Redis route did not observe the tunnel crash: {error:?}"
            ),
            Ok(()) => assert!(
                Instant::now() < deadline,
                "Redis route did not observe the tunnel crash"
            ),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let listener = bind_released_loopback(port).await;
    let error = adapter
        .ping()
        .await
        .expect_err("Redis reused a loopback endpoint after tunnel failure");
    assert_eq!(error.code.as_deref(), Some("ssh_tunnel_unavailable"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock,
        "Redis connected to a replacement process on the old tunnel port"
    );

    connection.close().await;
    connector.shutdown().await;
}

async fn bind_released_loopback(port: u16) -> TcpListener {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                listener.set_nonblocking(true).unwrap();
                return listener;
            }
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => panic!("could not claim the released SSH port {port}: {error}"),
        }
    }
}
