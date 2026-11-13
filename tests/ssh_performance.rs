#![cfg(unix)]

#[path = "fixtures/ssh/test_support.rs"]
mod support;

use std::{path::Path, sync::Arc, time::Instant};

use futures_util::TryStreamExt;
use lazydb::{
    connections::DatabaseConnector,
    db::DatabaseConnection,
    profile::{PgConnectionOptions, SshOptions, SslMode, import_connection_url},
};
use secrecy::SecretString;
use serde_json::json;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use support::{POSTGRES_PASSWORD_ENV, SshForwarder, fixture, required_env};

const ROUNDS: usize = 3;

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn summary(runs: &[serde_json::Value]) -> serde_json::Value {
    let mut qps = runs
        .iter()
        .map(|run| run["queries_per_second"].as_f64().unwrap())
        .collect::<Vec<_>>();
    let mut p50 = runs
        .iter()
        .map(|run| run["p50_ms"].as_f64().unwrap())
        .collect::<Vec<_>>();
    let mut p95 = runs
        .iter()
        .map(|run| run["p95_ms"].as_f64().unwrap())
        .collect::<Vec<_>>();
    json!({
        "median_queries_per_second": median(&mut qps),
        "median_p50_ms": median(&mut p50),
        "median_p95_ms": median(&mut p95),
        "runs": runs,
    })
}

async fn measure(
    connections: &[Arc<DatabaseConnection>],
    sql: &str,
    iterations: usize,
) -> serde_json::Value {
    for connection in connections {
        connection.execute(sql).await.unwrap();
    }
    let started = Instant::now();
    let tasks = connections.iter().map(|connection| async move {
        let mut samples = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let started = Instant::now();
            let outcome = connection.execute(sql).await.unwrap();
            assert!(outcome.stats.row_count > 0);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples
    });
    let mut samples = futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let elapsed = started.elapsed().as_secs_f64();
    samples.sort_by(f64::total_cmp);
    json!({
        "sessions": connections.len(),
        "samples": samples.len(),
        "elapsed_seconds": elapsed,
        "queries_per_second": samples.len() as f64 / elapsed,
        "p50_ms": samples[samples.len() / 2],
        "p95_ms": samples[(samples.len() * 95 / 100).min(samples.len() - 1)],
    })
}

fn raw_postgres_options(
    fixture: &support::FixtureManifest,
    password: &str,
    host: &str,
    port: u16,
    socket: Option<&Path>,
) -> PgConnectOptions {
    let mut options = PgConnectOptions::new()
        .host(host)
        .port(port)
        .username(&fixture.postgres.user)
        .database(&fixture.postgres.database)
        .password(password)
        .ssl_mode(PgSslMode::Require)
        .ssl_root_cert(&fixture.postgres.ca_file)
        .ssl_client_cert(&fixture.postgres.client_cert)
        .ssl_client_key(&fixture.postgres.client_key);
    if let Some(socket) = socket {
        options = options.socket(socket);
    }
    options
}

async fn raw_postgres_pool(options: PgConnectOptions) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap()
}

async fn measure_raw(pools: &[PgPool], sql: &str, iterations: usize) -> serde_json::Value {
    for pool in pools {
        let mut rows = sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).fetch_many(pool);
        while rows.try_next().await.unwrap().is_some() {}
    }
    let started = Instant::now();
    let tasks = pools.iter().map(|pool| async move {
        let mut samples = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let started = Instant::now();
            let mut count = 0;
            let mut rows = sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).fetch_many(pool);
            while rows.try_next().await.unwrap().is_some() {
                count += 1;
            }
            assert!(count > 0);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples
    });
    let mut samples = futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let elapsed = started.elapsed().as_secs_f64();
    samples.sort_by(f64::total_cmp);
    json!({
        "sessions": pools.len(),
        "samples": samples.len(),
        "elapsed_seconds": elapsed,
        "queries_per_second": samples.len() as f64 / elapsed,
        "p50_ms": samples[samples.len() / 2],
        "p95_ms": samples[(samples.len() * 95 / 100).min(samples.len() - 1)],
    })
}

#[tokio::test]
#[ignore = "opt-in transport benchmark; requires the SSH fixture environment"]
async fn compare_unix_and_tcp_forwarding_overhead() {
    let fixture = fixture();
    let password = required_env(POSTGRES_PASSWORD_ENV);
    let socket_directory = support::fixture_tempdir();
    let socket_path = socket_directory
        .path()
        .join(format!(".s.PGSQL.{}", fixture.postgres.port));
    let unix_forwarder = SshForwarder::start_unix(
        &fixture.ssh,
        &socket_path,
        &fixture.postgres.host,
        fixture.postgres.port,
    )
    .await;
    let tcp_forwarder =
        SshForwarder::start_tcp(&fixture.ssh, &fixture.postgres.host, fixture.postgres.port).await;
    let unix_options = raw_postgres_options(
        &fixture,
        &password,
        &fixture.postgres.host,
        fixture.postgres.port,
        socket_path.parent(),
    );
    let tcp_options = raw_postgres_options(
        &fixture,
        &password,
        "127.0.0.1",
        tcp_forwarder.tcp_addr().port(),
        None,
    );
    let mut unix_pools = vec![raw_postgres_pool(unix_options.clone()).await];
    let mut tcp_pools = vec![raw_postgres_pool(tcp_options.clone()).await];
    for sessions in [1, 4, 16] {
        while unix_pools.len() < sessions {
            unix_pools.push(raw_postgres_pool(unix_options.clone()).await);
            tcp_pools.push(raw_postgres_pool(tcp_options.clone()).await);
        }
        for (name, sql, iterations) in [
            ("small", "SELECT 42::bigint", 40),
            (
                "large",
                "SELECT value, repeat('x', 128) FROM generate_series(1, 10000) AS value",
                4,
            ),
        ] {
            let mut unix_runs = Vec::with_capacity(ROUNDS);
            let mut tcp_runs = Vec::with_capacity(ROUNDS);
            for round in 0..ROUNDS {
                if round % 2 == 0 {
                    unix_runs.push(measure_raw(&unix_pools, sql, iterations).await);
                    tcp_runs.push(measure_raw(&tcp_pools, sql, iterations).await);
                } else {
                    tcp_runs.push(measure_raw(&tcp_pools, sql, iterations).await);
                    unix_runs.push(measure_raw(&unix_pools, sql, iterations).await);
                }
            }
            println!(
                "SSH_BENCHMARK_RAW {}",
                json!({
                    "workload": name,
                    "unix": summary(&unix_runs),
                    "tcp": summary(&tcp_runs),
                })
            );
        }
    }
    for pool in unix_pools.into_iter().chain(tcp_pools) {
        pool.close().await;
    }
    unix_forwarder.shutdown().await;
    tcp_forwarder.shutdown().await;
}

#[tokio::test]
#[ignore = "opt-in network benchmark; requires the SSH fixture environment"]
async fn compare_managed_and_external_ssh_query_overhead() {
    let fixture = fixture();
    let mut profile = import_connection_url(
        &format!(
            "postgresql://{}@{}:{}/{}",
            fixture.postgres.user,
            fixture.postgres.host,
            fixture.postgres.port,
            fixture.postgres.database
        ),
        Some("ssh-performance"),
    )
    .unwrap()
    .profile;
    profile.ssl_mode = SslMode::Require;
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
        config_file: Some(fixture.ssh.config_file.clone()),
        known_hosts_file: Some(fixture.ssh.known_hosts_file.clone()),
        proxy_jump: None,
    });
    let password = SecretString::from(required_env(POSTGRES_PASSWORD_ENV));
    let connector = DatabaseConnector::new();
    let started = Instant::now();
    let mut managed_connections = vec![Arc::new(
        connector.connect(&profile, Some(&password)).await.unwrap(),
    )];
    let managed_cold_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let forwarder =
        SshForwarder::start_tcp(&fixture.ssh, &fixture.postgres.host, fixture.postgres.port).await;
    let mut external = profile.clone();
    external.ssh = None;
    external.host = Some("127.0.0.1".into());
    external.port = Some(forwarder.tcp_addr().port());
    let mut external_connections = vec![Arc::new(
        DatabaseConnection::connect(&external, Some(&password))
            .await
            .unwrap(),
    )];
    let external_cold_ms = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "SSH_BENCHMARK {}",
        json!({
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "tls": "require with client certificate",
            "external_cold_ms": external_cold_ms,
            "managed_cold_ms": managed_cold_ms,
        })
    );
    for sessions in [1, 4, 16] {
        while managed_connections.len() < sessions {
            managed_connections.push(Arc::new(
                connector.connect(&profile, Some(&password)).await.unwrap(),
            ));
            external_connections.push(Arc::new(
                DatabaseConnection::connect(&external, Some(&password))
                    .await
                    .unwrap(),
            ));
        }
        assert_eq!(connector.active_tunnel_count(), 1);
        for (name, sql, iterations) in [
            ("small", "SELECT 42::bigint", 40),
            (
                "large",
                "SELECT value, repeat('x', 128) FROM generate_series(1, 10000) AS value",
                4,
            ),
        ] {
            let mut external_runs = Vec::with_capacity(ROUNDS);
            let mut managed_runs = Vec::with_capacity(ROUNDS);
            for round in 0..ROUNDS {
                if round % 2 == 0 {
                    external_runs.push(measure(&external_connections, sql, iterations).await);
                    managed_runs.push(measure(&managed_connections, sql, iterations).await);
                } else {
                    managed_runs.push(measure(&managed_connections, sql, iterations).await);
                    external_runs.push(measure(&external_connections, sql, iterations).await);
                }
            }
            println!(
                "SSH_BENCHMARK {}",
                json!({
                    "workload": name,
                    "external": summary(&external_runs),
                    "managed": summary(&managed_runs),
                })
            );
        }
    }
    for connection in managed_connections.into_iter().chain(external_connections) {
        Arc::try_unwrap(connection).unwrap().close().await;
    }
    connector.shutdown().await;
    forwarder.shutdown().await;
}
