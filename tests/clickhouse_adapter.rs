use lazydb::{
    db::{DatabaseConnection, value::CellValue},
    profile::{
        CatalogScope, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy, DatabaseKind,
        Environment, ProfileAccess, SslMode, import_connection_url,
    },
};
use secrecy::SecretString;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
use uuid::Uuid;

#[path = "support/clickhouse.rs"]
mod clickhouse_support;
#[allow(dead_code)]
#[path = "support/mod.rs"]
mod support;

fn profile(url: &str) -> (ConnectionProfile, Option<SecretString>) {
    let imported = import_connection_url(url, Some("clickhouse-test")).unwrap();
    (imported.profile, imported.transient_password)
}

#[tokio::test]
async fn clickhouse_adapter_connects_over_http_and_probes_server_identity() {
    let Some(url) = support::clickhouse_test_url() else {
        return;
    };

    let (profile, password) = profile(&url);
    let connection = DatabaseConnection::connect(&profile, password.as_ref())
        .await
        .expect("connect to ClickHouse");
    let server = connection.probe().await.expect("probe ClickHouse");

    assert_eq!(server.kind, DatabaseKind::ClickHouse);
    assert!(!server.version.is_empty());
    assert!(!server.database.is_empty());
    assert!(server.current_user.is_some());
    connection.close().await;
}

#[tokio::test]
async fn clickhouse_select_preserves_typed_values_and_zero_row_metadata() {
    let Some(url) = support::clickhouse_test_url() else {
        return;
    };
    let (profile, password) = profile(&url);
    let connection = DatabaseConnection::connect(&profile, password.as_ref())
        .await
        .expect("connect to ClickHouse");
    let typed = connection
        .execute("SELECT toUInt64(1) AS n, '' AS empty, CAST(NULL AS Nullable(String)) AS missing")
        .await
        .unwrap();
    assert_eq!(typed.result_sets[0].columns.len(), 3);
    assert_eq!(typed.result_sets[0].rows.len(), 1);
    assert!(matches!(
        typed.result_sets[0].rows[0][0],
        CellValue::Unsigned(1)
    ));
    assert!(
        matches!(typed.result_sets[0].rows[0][1], CellValue::Text(ref value) if value.is_empty())
    );
    assert!(matches!(typed.result_sets[0].rows[0][2], CellValue::Null));

    let empty = connection
        .execute("SELECT toUInt64(number) AS n FROM numbers(0)")
        .await
        .unwrap();
    assert_eq!(empty.result_sets.len(), 1);
    assert!(empty.result_sets[0].rows.is_empty());
    assert_eq!(empty.result_sets[0].columns[0].name, "n");
    assert_eq!(empty.result_sets[0].columns[0].type_name, "UInt64");
    connection.close().await;
}

#[tokio::test]
async fn clickhouse_live_integration_uses_an_isolated_database() {
    let Some(url) = support::clickhouse_test_url() else {
        return;
    };
    let (mut profile, password) = profile(&url);
    let connection = DatabaseConnection::connect(&profile, password.as_ref())
        .await
        .expect("connect to ClickHouse");
    let database = clickhouse_support::test_database_name();
    clickhouse_support::create_test_database(&connection, &database)
        .await
        .expect("create isolated ClickHouse test database");
    if let lazydb::profile::CatalogSelection::Selected(databases) =
        &mut profile.catalog_scope.databases
    {
        databases.push(lazydb::profile::DatabaseScope {
            name: database.clone(),
            schemas: lazydb::profile::CatalogSelection::All,
        });
    }
    let target = lazydb::model::execution_target::ExecutionTarget {
        profile_id: profile.id,
        database: database.clone(),
        schema: Some(database.clone()),
    };
    let isolated = DatabaseConnection::connect_target(&profile, password.as_ref(), &target)
        .await
        .expect("connect to isolated ClickHouse database");
    let query = isolated
        .execute("SELECT currentDatabase() AS db")
        .await
        .expect("execute query in isolated database");
    assert!(
        matches!(query.result_sets[0].rows[0][0], CellValue::Text(ref name) if name == &database)
    );
    isolated.close().await;
    clickhouse_support::drop_test_database(&connection, &database)
        .await
        .expect("drop isolated ClickHouse test database");
    connection.close().await;
}

#[test]
fn fixture_profile_type_contract_is_explicit() {
    // Keeps the test profile contract visible to future mock transport tests.
    let profile = ConnectionProfile {
        id: Uuid::new_v4(),
        name: "fixture".into(),
        access: ProfileAccess::Global,
        group_id: None,
        kind: DatabaseKind::ClickHouse,
        url_format: ConnectionUrlFormat::ClickHouse,
        host: Some("127.0.0.1".into()),
        port: Some(8123),
        user: Some("default".into()),
        database: Some("default".into()),
        default_schema: Some("default".into()),
        sqlite_path: None,
        snowflake_options: None,
        duckdb_path: None,
        mongodb_options: None,
        elasticsearch_options: None,
        pg_options: None,
        ssl_mode: SslMode::Disable,
        credential_policy: CredentialPolicy::None,
        read_only: false,
        environment: Environment::Development,
        catalog_scope: CatalogScope::for_profile(DatabaseKind::ClickHouse, "default", None),
    };
    assert_eq!(profile.kind, DatabaseKind::ClickHouse);
}

#[tokio::test]
async fn mock_probe_uses_sensitive_auth_headers_and_body_query() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (server_tx, server_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|length| length.parse::<usize>().ok())
                .unwrap_or(0);
            if content_length > 0 && request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let request_body = String::from_utf8_lossy(&request[header_end + 4..]);
        let _ = server_tx.send((
            headers.clone(),
            String::from_utf8_lossy(&request[header_end + 4..]).to_string(),
        ));
        assert!(headers.contains("x-clickhouse-user: alice"));
        assert!(headers.contains("x-clickhouse-key: hidden-value"));
        assert!(headers.contains("database=analytics"));
        assert!(!headers.contains("hidden-value@"));
        assert!(request_body.contains("SELECT version()"));
        let response_body = b"25.8.1\tanalytics\talice\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response_body.len()
        );
        write!(stream, "{response}").unwrap();
        stream.write_all(response_body).unwrap();
    });
    let (profile, _) = profile(&format!("clickhouse://alice@{address}/analytics"));
    let password = SecretString::from("hidden-value".to_owned());
    let adapter = lazydb::db::clickhouse::ClickHouseAdapter::connect(&profile, Some(&password))
        .await
        .unwrap_or_else(|error| {
            panic!(
                "connect failed: {error}; server: {:?}",
                server_rx.try_recv()
            )
        });
    adapter.close().await;
    server.join().unwrap();
}

#[tokio::test]
async fn mock_http_errors_are_classified_and_redirects_are_not_followed() {
    for (status, expected_category) in [
        (
            "401 Unauthorized",
            lazydb::db::ErrorCategory::Authentication,
        ),
        ("403 Forbidden", lazydb::db::ErrorCategory::Permission),
        ("500 Internal Server Error", lazydb::db::ErrorCategory::Sql),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            let body = "mock failure";
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let (profile, _) = profile(&format!(
            "clickhouse://127.0.0.1:{}/analytics",
            address.port()
        ));
        let error = lazydb::db::clickhouse::ClickHouseAdapter::connect(&profile, None)
            .await
            .unwrap_err();
        assert_eq!(error.category, expected_category);
        assert!(error.message.contains("mock failure"));
        server.join().unwrap();
    }

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request).unwrap();
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let (profile, _) = profile(&format!(
        "clickhouse://127.0.0.1:{}/analytics",
        address.port()
    ));
    let error = lazydb::db::clickhouse::ClickHouseAdapter::connect(&profile, None)
        .await
        .unwrap_err();
    assert_eq!(error.category, lazydb::db::ErrorCategory::Sql);
    server.join().unwrap();
}
