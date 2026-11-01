use super::*;
use crate::db::query::QueryBudget;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

const HANDLE: &str = "536fad38-b564-4dc5-9892-a4543504df6c";

fn result(rows: Value, count: u64, partitions: usize) -> Value {
    json!({"statementHandle": HANDLE, "resultSetMetaData": {
        "numRows": count, "rowType": [{"name": "ID", "type": "fixed", "scale": 0, "precision": 38}],
        "partitionInfo": (0..partitions).map(|_| json!({"rowCount": 1})).collect::<Vec<_>>()
    }, "data": rows})
}

fn mock(
    responses: Vec<(u16, Value)>,
) -> (
    SnowflakeAdapter,
    mpsc::Receiver<String>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        for (status, response) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let len = stream.read(&mut buf).unwrap();
                assert_ne!(len, 0);
                request.extend_from_slice(&buf[..len]);
                if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let body_len = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + body_len {
                        break;
                    }
                }
            }
            let _ = sender.send(String::from_utf8(request).unwrap());
            if response.get("mock_gzip").is_some() {
                // gzip-compressed {"data":[["2"]]}, as returned by subsequent SQL API partitions.
                let body = [
                    31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 171, 86, 74, 73, 44, 73, 84, 178, 138, 142,
                    86, 50, 82, 138, 141, 173, 5, 0, 13, 220, 21, 190, 16, 0, 0, 0,
                ];
                write!(stream, "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            } else {
                let body = response.to_string();
                write!(stream, "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        }
    });
    let adapter = SnowflakeAdapter {
        client: Client::builder().no_proxy().build().unwrap(),
        endpoint: url::Url::parse(&format!("http://{address}/api/v2/statements")).unwrap(),
        token: SecretString::from("test-access-token"),
        token_type: "PROGRAMMATIC_ACCESS_TOKEN".into(),
        profile_id: Uuid::new_v4(),
        database: "ANALYTICS".into(),
        schema: Some("PUBLIC".into()),
        warehouse: Some("COMPUTE_WH".into()),
        role: Some("ANALYST".into()),
        catalog_scope: CatalogScope::for_profile(
            DatabaseKind::Snowflake,
            "ANALYTICS",
            Some("PUBLIC"),
        ),
    };
    (adapter, receiver, worker)
}

#[tokio::test]
async fn decodes_gzip_compressed_result_partitions() {
    let (adapter, _, worker) = mock(vec![
        (200, result(json!([["1"]]), 2, 2)),
        (200, json!({"mock_gzip": true})),
    ]);
    let outcome = adapter.execute("SELECT ID FROM T").await.unwrap();
    assert_eq!(outcome.result_sets[0].rows[1], vec![CellValue::Integer(2)]);
    worker.join().unwrap();
}

#[test]
fn validates_saved_profile_transport_options_before_network_access() {
    let imported = crate::profile::import_connection_url(
        "snowflake://user@org-account.snowflakecomputing.com/ANALYTICS",
        None,
    )
    .unwrap();
    let mut profile = imported.profile;
    let token = SecretString::from("access-token");
    assert!(SnowflakeAdapter::from_profile(&profile, Some(&token)).is_ok());
    assert!(SnowflakeAdapter::from_profile(&profile, None).is_err());
    profile.port = Some(80);
    assert!(SnowflakeAdapter::from_profile(&profile, Some(&token)).is_err());
    profile.port = Some(443);
    profile.ssl_mode = SslMode::Disable;
    assert!(SnowflakeAdapter::from_profile(&profile, Some(&token)).is_err());
    profile.ssl_mode = SslMode::VerifyFull;
    profile.snowflake_options.as_mut().unwrap().warehouse = Some(" ".into());
    assert!(SnowflakeAdapter::from_profile(&profile, Some(&token)).is_err());
}

#[tokio::test]
async fn submits_context_and_bearer_then_polls_the_trusted_endpoint() {
    let (adapter, requests, worker) = mock(vec![
        (
            202,
            json!({"statementHandle": HANDLE, "statementStatusUrl": "https://untrusted.invalid/steal"}),
        ),
        (429, json!({"statementHandle": HANDLE})),
        (200, result(json!([["42"]]), 1, 1)),
    ]);
    let outcome = adapter.execute("SELECT 42").await.unwrap();
    assert_eq!(outcome.result_sets[0].rows[0][0], CellValue::Integer(42));
    let request = requests.recv().unwrap();
    assert!(request.starts_with("POST /api/v2/statements?requestId="));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-access-token")
    );
    assert!(request.contains("PROGRAMMATIC_ACCESS_TOKEN"));
    let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["warehouse"], "COMPUTE_WH");
    assert_eq!(body["role"], "ANALYST");
    assert_eq!(body["schema"], "PUBLIC");
    assert_eq!(body["parameters"]["MULTI_STATEMENT_COUNT"], "1");
    assert!(body["parameters"].get("AUTOCOMMIT").is_none());
    assert_eq!(body["parameters"]["BINARY_OUTPUT_FORMAT"], "HEX");
    assert_eq!(body["statement"], "SELECT 42");
    for _ in 0..2 {
        assert!(
            requests
                .recv()
                .unwrap()
                .starts_with(&format!("GET /api/v2/statements/{HANDLE} "))
        );
    }
    worker.join().unwrap();
}

#[tokio::test]
async fn fetches_partitions_with_initial_metadata_and_distinguishes_null() {
    let (adapter, requests, worker) = mock(vec![
        (200, result(json!([["1"]]), 3, 3)),
        (200, json!({"data": [[null]]})),
        (200, json!({"data": [["3"]]})),
    ]);
    let outcome = adapter.execute("SELECT ID FROM T").await.unwrap();
    assert_eq!(
        outcome.result_sets[0].rows,
        vec![
            vec![CellValue::Integer(1)],
            vec![CellValue::Null],
            vec![CellValue::Integer(3)]
        ]
    );
    assert!(!outcome.stats.truncated);
    requests.recv().unwrap();
    for number in 1..=2 {
        assert!(requests.recv().unwrap().starts_with(&format!(
            "GET /api/v2/statements/{HANDLE}?partition={number} "
        )));
    }
    worker.join().unwrap();
}

#[tokio::test]
async fn budget_stops_before_requesting_unneeded_partitions() {
    let (adapter, _, worker) = mock(vec![(200, result(json!([["1"]]), 3, 3))]);
    let outcome = adapter
        .execute_with_budget(
            "SELECT ID FROM T",
            QueryBudget {
                max_rows: 1,
                max_bytes: usize::MAX,
            },
        )
        .await
        .unwrap();
    assert_eq!(outcome.stats.row_count, 1);
    assert!(outcome.stats.truncated);
    worker.join().unwrap();
}

#[tokio::test]
async fn byte_budget_retains_only_a_contiguous_prefix() {
    let (adapter, _, worker) = mock(vec![(200, result(json!([["1"], ["2"]]), 2, 1))]);
    let outcome = adapter
        .execute_with_budget(
            "SELECT ID FROM T",
            QueryBudget {
                max_rows: 10,
                max_bytes: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(outcome.stats.row_count, 0);
    assert!(outcome.stats.truncated);
    worker.join().unwrap();
}

#[tokio::test]
async fn reports_sql_failure_without_replaying_the_statement() {
    let (adapter, requests, worker) = mock(vec![(
        422,
        json!({"code": "002003", "sqlState": "42501", "message": "insufficient privileges"}),
    )]);
    let error = adapter.execute("DELETE FROM T").await.unwrap_err();
    assert_eq!(error.category, ErrorCategory::Permission);
    assert_eq!(error.code.as_deref(), Some("002003"));
    assert!(requests.recv().unwrap().starts_with("POST "));
    worker.join().unwrap();
}

#[tokio::test]
async fn expired_and_revoked_tokens_are_authentication_errors_without_replay() {
    for token_type in ["OAUTH", "PROGRAMMATIC_ACCESS_TOKEN"] {
        for (status, code) in [(401, "390318"), (401, "390303"), (500, "390111")] {
            let (mut adapter, requests, worker) = mock(vec![(
                status,
                json!({"code": code, "message": "Access token expired or revoked"}),
            )]);
            adapter.token_type = token_type.into();
            let error = adapter.execute("SELECT 42").await.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Authentication);
            assert_eq!(error.code.as_deref(), Some(code));
            assert!(!error.message.contains("test-access-token"));
            worker.join().unwrap();
            let requests = requests.into_iter().collect::<Vec<_>>();
            assert_eq!(requests.len(), 1);
            assert!(requests[0].contains(token_type));
        }
    }
    assert_eq!(
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &json!({"code": "unrelated"})
        )
        .category,
        ErrorCategory::Network
    );
}

fn text_result(names: &[&str], rows: Value) -> Value {
    json!({"statementHandle": HANDLE, "resultSetMetaData": {
        "numRows": rows.as_array().unwrap().len(),
        "rowType": names.iter().map(|name| json!({"name": name, "type": "text"})).collect::<Vec<_>>()
    }, "data": rows})
}

#[test]
fn saved_profile_without_schema_submits_public_and_preserves_explicit_schema() {
    let mut profile = crate::profile::import_connection_url(
        "snowflake://user@org-account.snowflakecomputing.com/ANALYTICS",
        None,
    )
    .unwrap()
    .profile;
    let token = SecretString::from("access-token");
    for (saved, expected) in [(None, "PUBLIC"), (Some("MixedCase"), "MixedCase")] {
        profile.default_schema = saved.map(str::to_owned);
        let adapter = SnowflakeAdapter::from_profile(&profile, Some(&token)).unwrap();
        assert_eq!(adapter.statement_body("SELECT 1")["schema"], expected);
    }
}

#[tokio::test]
async fn every_statement_uses_supported_parameters_and_decodes_hex_binary() {
    let (adapter, requests, worker) = mock(vec![
        (
            200,
            text_result(&["status"], json!([["Statement executed successfully."]])),
        ),
        (
            200,
            json!({"resultSetMetaData": {
            "numRows": 1, "rowType": [{"name": "B", "type": "binary"}]
        }, "data": [["00ABFF"]]}),
        ),
    ]);
    adapter.execute("CREATE TABLE T (B BINARY)").await.unwrap();
    let outcome = adapter.execute("SELECT B FROM T").await.unwrap();
    assert_eq!(
        outcome.result_sets[0].rows[0][0],
        CellValue::Bytes(vec![0, 171, 255])
    );
    for _ in 0..2 {
        let request = requests.recv().unwrap();
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(body["parameters"].get("AUTOCOMMIT").is_none());
        assert_eq!(body["parameters"]["BINARY_OUTPUT_FORMAT"], "HEX");
    }
    worker.join().unwrap();
}

#[tokio::test]
async fn discovery_ignores_saved_selection_while_browsing_remains_scoped() {
    use crate::{
        db::catalog::{CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogTarget},
        identity::ConnectionIdentity,
    };
    let (adapter, requests, worker) = mock(vec![
        (
            200,
            text_result(&["name"], json!([["ANALYTICS"], ["OTHER"]])),
        ),
        (
            200,
            text_result(
                &["SCHEMA_NAME", "COMMENT"],
                json!([["PRIVATE", null], ["PUBLIC", null]]),
            ),
        ),
        (
            200,
            text_result(&["SCHEMA_NAME", "COMMENT"], json!([["RAW", null]])),
        ),
    ]);
    let discovery = adapter.discover_catalog_scope().await.unwrap();
    assert!(discovery.warnings.is_empty());
    assert_eq!(discovery.databases.len(), 2);
    assert_eq!(discovery.databases[0].schemas, ["PRIVATE", "PUBLIC"]);
    assert_eq!(discovery.databases[1].name, "OTHER");
    assert_eq!(discovery.databases[1].schemas, ["RAW"]);
    let statements = (0..3).map(|_| {
        let request = requests.recv().unwrap();
        serde_json::from_str::<Value>(request.split_once("\r\n\r\n").unwrap().1).unwrap()["statement"]
            .as_str().unwrap().to_owned()
    }).collect::<Vec<_>>();
    assert_eq!(statements[0], "SHOW DATABASES LIMIT 501");
    assert!(statements[1].contains("WHERE TRUE ORDER BY SCHEMA_NAME"));
    assert!(statements[2].contains("\"OTHER\".\"INFORMATION_SCHEMA\".\"SCHEMATA\" WHERE TRUE"));
    let mut request = CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id: adapter.profile_id,
                generation: 1,
            },
            catalog_epoch: 0,
            request_id: 1,
            target: CatalogTarget::Databases,
            cursor: None,
        },
        scope: CatalogScope {
            databases: crate::profile::CatalogSelection::All,
        },
        page_size: 10,
    };
    let page = adapter.load_catalog_page(&request).await.unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].qualified_name.object, "ANALYTICS");
    request.key.target = CatalogTarget::Schemas {
        database: CatalogId::new(adapter.profile_id, CatalogKind::Database, ["OTHER"]),
    };
    assert_eq!(
        adapter
            .load_catalog_page(&request)
            .await
            .unwrap_err()
            .category,
        ErrorCategory::Configuration
    );
    let relation = CatalogId::new(
        adapter.profile_id,
        CatalogKind::Table,
        ["ANALYTICS", "PRIVATE", "T"],
    );
    assert_eq!(
        adapter
            .resolve_relation_identity(&relation)
            .await
            .unwrap_err()
            .category,
        ErrorCategory::Configuration
    );
    worker.join().unwrap();
}

#[tokio::test]
async fn discovery_retains_other_databases_when_schema_access_fails() {
    let (adapter, _, worker) = mock(vec![
        (
            200,
            text_result(&["name"], json!([["A"], ["DENIED"], ["DROPPED"], ["Z"]])),
        ),
        (
            200,
            text_result(&["SCHEMA_NAME", "COMMENT"], json!([["PUBLIC", null]])),
        ),
        (
            422,
            json!({"sqlState": "42501", "message": "insufficient privileges"}),
        ),
        (
            422,
            json!({"sqlState": "02000", "code": "002003", "message": "database no longer exists"}),
        ),
        (
            200,
            text_result(&["SCHEMA_NAME", "COMMENT"], json!([["RAW", null]])),
        ),
    ]);
    let discovery = adapter.discover_catalog_scope().await.unwrap();
    assert_eq!(
        discovery
            .databases
            .iter()
            .map(|db| db.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "DENIED", "DROPPED", "Z"]
    );
    assert_eq!(discovery.databases[0].schemas, ["PUBLIC"]);
    assert!(discovery.databases[1].schemas.is_empty());
    assert!(discovery.databases[2].schemas.is_empty());
    assert_eq!(discovery.databases[3].schemas, ["RAW"]);
    assert_eq!(discovery.warnings.len(), 2);
    assert!(discovery.warnings[0].contains("DENIED"));
    assert!(discovery.warnings[0].contains("insufficient privileges"));
    assert!(discovery.warnings[1].contains("DROPPED"));
    worker.join().unwrap();
}

#[tokio::test]
async fn discovery_keeps_completed_schema_pages_on_later_failure() {
    let rows = (0..501)
        .map(|index| json!([format!("S{index:03}"), null]))
        .collect::<Vec<_>>();
    let (adapter, requests, worker) = mock(vec![
        (200, text_result(&["name"], json!([["DB"]]))),
        (200, text_result(&["SCHEMA_NAME", "COMMENT"], json!(rows))),
        (403, json!({"message": "access revoked"})),
    ]);
    let discovery = adapter.discover_catalog_scope().await.unwrap();
    assert_eq!(discovery.databases[0].schemas.len(), 500);
    assert_eq!(discovery.databases[0].schemas.last().unwrap(), "S499");
    assert_eq!(discovery.warnings.len(), 1);
    let last = requests.into_iter().last().unwrap();
    let body: Value = serde_json::from_str(last.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert!(
        body["statement"]
            .as_str()
            .unwrap()
            .contains("SCHEMA_NAME > 'S499'")
    );
    worker.join().unwrap();
}

#[tokio::test]
async fn catalog_preview_uses_full_relation_name_and_lookahead() {
    use crate::{
        db::catalog::{CatalogId, CatalogKind},
        model::{
            pagination::{PageRequest, PageSize},
            relation::RelationPreviewOptions,
        },
    };
    let (adapter, requests, worker) = mock(vec![
        (
            200,
            text_result(
                &["TABLE_NAME", "TABLE_TYPE", "COMMENT"],
                json!([["Orders", "BASE TABLE", null]]),
            ),
        ),
        (200, result(json!([["1"], ["2"]]), 2, 1)),
    ]);
    let relation = CatalogId::new(
        adapter.profile_id,
        CatalogKind::Table,
        ["ANALYTICS", "PUBLIC", "Orders"],
    );
    let preview = adapter
        .preview_relation_with_scope(
            &relation,
            &adapter.catalog_scope,
            &RelationPreviewOptions::default(),
            PageRequest::at(PageSize::Ten, 20),
        )
        .await
        .unwrap();
    assert_eq!(preview.pagination.offset, 20);
    assert_eq!(preview.pagination.visible_rows, 2);
    assert!(!preview.pagination.has_next);
    requests.recv().unwrap();
    let request = requests.recv().unwrap();
    let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body["statement"],
        "SELECT * FROM \"ANALYTICS\".\"PUBLIC\".\"Orders\" LIMIT 11 OFFSET 20"
    );
    worker.join().unwrap();
}

#[tokio::test]
async fn catalog_ddl_reads_native_ddl_and_columns() {
    use crate::db::catalog::{CatalogId, CatalogKind};
    let (adapter, requests, worker) = mock(vec![
        (
            200,
            text_result(
                &["TABLE_NAME", "TABLE_TYPE", "COMMENT"],
                json!([["Orders", "BASE TABLE", null]]),
            ),
        ),
        (
            200,
            text_result(&["GET_DDL"], json!([["CREATE TABLE Orders (ID NUMBER);"]])),
        ),
        (
            200,
            text_result(
                &["TABLE_NAME", "TABLE_TYPE", "COMMENT"],
                json!([["Orders", "BASE TABLE", null]]),
            ),
        ),
        (
            200,
            text_result(
                &[
                    "COLUMN_NAME",
                    "DATA_TYPE",
                    "ORDINAL_POSITION",
                    "IS_NULLABLE",
                    "COLUMN_DEFAULT",
                    "COMMENT",
                    "IS_IDENTITY",
                    "NUMERIC_PRECISION",
                    "NUMERIC_SCALE",
                    "CHARACTER_MAXIMUM_LENGTH",
                    "COLLATION_NAME",
                    "CHARACTER_SET_NAME",
                ],
                json!([[
                    "ID", "NUMBER", "1", "NO", null, null, "NO", "38", "0", null, null, null
                ]]),
            ),
        ),
    ]);
    let relation = CatalogId::new(
        adapter.profile_id,
        CatalogKind::Table,
        ["ANALYTICS", "PUBLIC", "Orders"],
    );
    let ddl = adapter
        .relation_ddl_with_scope(&relation, &adapter.catalog_scope)
        .await
        .unwrap();
    assert!(ddl.sql.starts_with("CREATE TABLE"));
    assert_eq!(ddl.children.entries.len(), 1);
    requests.recv().unwrap();
    let request = requests.recv().unwrap();
    let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body["statement"],
        "SELECT GET_DDL('TABLE', '\"ANALYTICS\".\"PUBLIC\".\"Orders\"', TRUE)"
    );
    worker.join().unwrap();
}

#[tokio::test]
async fn retries_a_throttled_partition_without_losing_its_index() {
    let (adapter, requests, worker) = mock(vec![
        (200, result(json!([["1"]]), 2, 2)),
        (429, json!({"code": "rate_limit"})),
        (200, json!({"data": [["2"]]})),
    ]);
    let result = adapter.execute("SELECT ID FROM T").await.unwrap();
    assert_eq!(result.stats.row_count, 2);
    requests.recv().unwrap();
    for _ in 0..2 {
        assert!(
            requests
                .recv()
                .unwrap()
                .starts_with(&format!("GET /api/v2/statements/{HANDLE}?partition=1 "))
        );
    }
    worker.join().unwrap();
}

#[tokio::test]
async fn rejects_inconsistent_partition_counts() {
    let (adapter, _, worker) = mock(vec![(200, result(json!([["1"]]), 2, 1))]);
    assert_eq!(
        adapter
            .execute("SELECT ID FROM T")
            .await
            .unwrap_err()
            .code
            .as_deref(),
        Some("snowflake_invalid_response")
    );
    worker.join().unwrap();
}
