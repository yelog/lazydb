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
    db::{DatabaseConnection, transaction::TransactionBackend, value::CellValue},
    model::execution_target::ExecutionTarget,
    profile::{ConnectionProfile, SshOptions, SslMode, import_connection_url},
};
use secrecy::SecretString;
use support::{fixture, kill_managed_ssh_tunnels, required_env};

fn profile() -> ConnectionProfile {
    let fixture = fixture();
    let mut profile = import_connection_url(
        "sqlserver://sa@mssql.internal:1433/tempdb",
        Some("sqlserver-ssh"),
    )
    .unwrap()
    .profile;
    profile.catalog_scope.databases = lazydb::profile::CatalogSelection::All;
    profile.ssl_mode = SslMode::Require;
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

#[tokio::test]
#[ignore = "requires the SSH fixture with --with-sqlserver"]
async fn sql_server_tunnel_covers_pool_reuse_transactions_and_database_targets() {
    let profile = profile();
    let password = SecretString::from(required_env("LAZYDB_TEST_SSH_SQLSERVER_PASSWORD"));
    let connector = DatabaseConnector::new();
    let connection = connector.connect(&profile, Some(&password)).await.unwrap();
    let probes = (0..12).map(|_| connection.execute("SELECT CAST(42 AS bigint) AS answer"));
    for result in futures_util::future::join_all(probes).await {
        assert_eq!(
            result.unwrap().result_sets[0].rows[0][0],
            CellValue::Integer(42)
        );
    }
    let table = format!("[dbo].[lazydb_ssh_{}]", uuid::Uuid::new_v4().simple());
    connection
        .execute(&format!("CREATE TABLE {table} ([id] int NOT NULL)"))
        .await
        .unwrap();
    let mut transaction = match &connection {
        DatabaseConnection::SqlServer(adapter) => adapter.transaction_backend().await.unwrap(),
        _ => panic!("expected SQL Server"),
    };
    transaction.begin().await.unwrap();
    transaction
        .execute(&format!("INSERT INTO {table} VALUES (1)"))
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    transaction.begin().await.unwrap();
    transaction
        .execute(&format!("INSERT INTO {table} VALUES (2)"))
        .await
        .unwrap();
    transaction.rollback().await.unwrap();
    transaction.force_close().await.unwrap();
    assert_eq!(
        connection
            .execute(&format!("SELECT COUNT_BIG(*) FROM {table}"))
            .await
            .unwrap()
            .result_sets[0]
            .rows[0][0],
        CellValue::Integer(1)
    );
    connection
        .execute(&format!("DROP TABLE {table}"))
        .await
        .unwrap();
    let target = ExecutionTarget {
        profile_id: profile.id,
        database: "master".into(),
        schema: Some("dbo".into()),
    };
    let second = connector
        .connect_target(&profile, Some(&password), &target)
        .await
        .unwrap();
    assert_eq!(second.probe().await.unwrap().database, "master");
    assert_eq!(connector.active_tunnel_count(), 1);
    connection.close().await;
    second.execute("SELECT 1").await.unwrap();
    second.close().await;
    connector.shutdown().await;
}

#[tokio::test]
#[ignore = "requires the SSH fixture with --with-sqlserver"]
async fn sql_server_failed_pool_cannot_reconnect_to_a_replacement_listener() {
    let fixture = fixture();
    let profile = profile();
    let password = SecretString::from(required_env("LAZYDB_TEST_SSH_SQLSERVER_PASSWORD"));
    let connector = DatabaseConnector::new();
    let connection = connector.connect(&profile, Some(&password)).await.unwrap();
    let tunnels = kill_managed_ssh_tunnels(&fixture.ssh.client_key);
    let port = tunnels.iter().find_map(|tunnel| tunnel.local_port).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if connection
            .probe()
            .await
            .is_err_and(|error| error.code.as_deref() == Some("ssh_tunnel_exited"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "SQL Server did not observe SSH failure"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let error = connection
        .execute("SELECT 1")
        .await
        .expect_err("failed SQL Server route was reused");
    assert_eq!(error.code.as_deref(), Some("ssh_tunnel_exited"));
    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    connection.close().await;
    connector.shutdown().await;
}
