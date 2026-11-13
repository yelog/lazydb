#![cfg(unix)]

#[path = "fixtures/ssh/test_support.rs"]
mod support;

use std::{
    io::ErrorKind,
    net::TcpListener,
    time::{Duration, Instant},
};

use lazydb::{
    connections::DatabaseConnector,
    db::{DatabaseConnection, ErrorCategory, query::QueryBudget},
    profile::{
        ConnectionProfile, ElasticsearchConnectionOptions, OpenSearchAuth,
        OpenSearchConnectionOptions, SshOptions, import_connection_url,
    },
};
use support::{FixtureManifest, fixture, kill_managed_ssh_tunnels};

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

fn profile(fixture: &FixtureManifest, url: &str, name: &str) -> ConnectionProfile {
    let mut profile = import_connection_url(url, Some(name)).unwrap().profile;
    profile.ssh = Some(ssh_options(fixture));
    profile
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn clickhouse_http_uses_socks_and_preserves_the_logical_endpoint() {
    let fixture = fixture();
    let profile = profile(
        &fixture,
        &fixture.http.clickhouse_url,
        "ssh-clickhouse-http",
    );
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, None)
        .await
        .expect("connect ClickHouse HTTP through managed SSH SOCKS");
    let adapter = match &connection {
        DatabaseConnection::ClickHouse(adapter) => adapter,
        _ => panic!("ClickHouse profile used the wrong adapter"),
    };

    let server = adapter.probe().await.unwrap();
    assert_eq!(server.version, "24.1.1.1");
    assert_eq!(server.database, "default");
    assert_eq!(server.current_user.as_deref(), Some("default"));

    connection.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn elasticsearch_https_uses_socks_with_strict_tls_and_original_authority() {
    let fixture = fixture();
    let mut profile = profile(
        &fixture,
        &fixture.http.elasticsearch_url,
        "ssh-elasticsearch-https",
    );
    profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: Default::default(),
        ca_cert_path: Some(fixture.http.ca_file.clone()),
    });
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, None)
        .await
        .expect("connect Elasticsearch HTTPS through managed SSH SOCKS");
    let adapter = match &connection {
        DatabaseConnection::Elasticsearch(adapter) => adapter,
        _ => panic!("Elasticsearch profile used the wrong adapter"),
    };

    let server = adapter.probe().await.unwrap();
    assert_eq!(server.version, "8.15.0");
    let outcome = adapter
        .execute_with_budget("GET /_cluster/health", QueryBudget::UNBOUNDED)
        .await
        .unwrap();
    assert_eq!(outcome.result_sets[0].rows.len(), 1);

    connection.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn opensearch_https_uses_socks_with_original_host_and_strict_tls() {
    let fixture = fixture();
    let mut profile = profile(
        &fixture,
        &fixture.http.opensearch_url,
        "ssh-opensearch-https",
    );
    profile.opensearch_options = Some(OpenSearchConnectionOptions {
        auth: OpenSearchAuth::None,
        ca_cert_path: Some(fixture.http.ca_file.clone()),
    });
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, None)
        .await
        .expect("connect OpenSearch HTTPS through managed SSH SOCKS");
    let adapter = match &connection {
        DatabaseConnection::OpenSearch(adapter) => adapter,
        _ => panic!("OpenSearch profile used the wrong adapter"),
    };

    let server = adapter.probe().await.unwrap();
    assert_eq!(server.version, "2.19.0");

    connection.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn https_socks_rejects_an_untrusted_certificate_authority() {
    let fixture = fixture();
    let mut profile = profile(
        &fixture,
        &fixture.http.elasticsearch_url,
        "ssh-elasticsearch-wrong-ca",
    );
    profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: Default::default(),
        ca_cert_path: Some(fixture.http.wrong_ca_file.clone()),
    });
    let connector = DatabaseConnector::new();

    let error = connector
        .connect(&profile, None)
        .await
        .expect_err("Elasticsearch accepted an unrelated CA through SOCKS");
    assert_eq!(error.category, ErrorCategory::Network);
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires scripts/ci/test-ssh-integration.py"]
async fn crashed_http_tunnel_fails_closed_before_reusing_its_loopback_port() {
    let fixture = fixture();
    let mut profile = profile(
        &fixture,
        &fixture.http.elasticsearch_url,
        "ssh-elasticsearch-tunnel-crash",
    );
    profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: Default::default(),
        ca_cert_path: Some(fixture.http.ca_file.clone()),
    });
    let connector = DatabaseConnector::new();
    let connection = connector
        .connect(&profile, None)
        .await
        .expect("connect Elasticsearch before tunnel crash");
    let adapter = match &connection {
        DatabaseConnection::Elasticsearch(adapter) => adapter,
        _ => panic!("Elasticsearch profile used the wrong adapter"),
    };
    let released = kill_managed_ssh_tunnels(&fixture.ssh.client_key);
    let port = released
        .iter()
        .find_map(|tunnel| tunnel.local_port)
        .expect("managed HTTP tunnel did not expose a loopback port");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match adapter
            .execute_with_budget("GET /_cluster/health", QueryBudget::UNBOUNDED)
            .await
        {
            Err(error) if error.code.as_deref() == Some("ssh_tunnel_exited") => {
                assert_eq!(error.category, ErrorCategory::Network);
                break;
            }
            Err(error) => assert!(
                Instant::now() < deadline,
                "HTTP route did not observe the tunnel crash: {error:?}"
            ),
            Ok(_) => assert!(
                Instant::now() < deadline,
                "HTTP route did not observe the tunnel crash"
            ),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let listener = bind_released_loopback(port).await;
    let error = adapter
        .execute_with_budget("GET /_cluster/health", QueryBudget::UNBOUNDED)
        .await
        .expect_err("HTTP driver reused a loopback endpoint after tunnel failure");
    assert_eq!(error.code.as_deref(), Some("ssh_tunnel_exited"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock,
        "HTTP driver connected to a replacement process on the old tunnel port"
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
