use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{CatalogId, CatalogKind},
    },
    model::{
        pagination::{PageRequest, PageSize, TotalRows},
        relation::RelationPreviewOptions,
    },
    profile::{ElasticsearchAuth, ElasticsearchConnectionOptions, SslMode, import_connection_url},
};
use serde_json::{Value, json};
use std::{
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }
}

fn server(responses: Vec<(u16, Value)>) -> (String, thread::JoinHandle<Vec<String>>) {
    raw_server(
        responses
            .into_iter()
            .map(|(status, body)| HttpResponse::new(status, body.to_string()))
            .collect(),
    )
}

fn raw_server(responses: Vec<HttpResponse>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "fixture request timed out");
                        thread::park_timeout(Duration::from_millis(1));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(request).unwrap());
            write!(
                stream,
                "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                response.status,
                response.body.len()
            )
            .unwrap();
            for (name, value) in response.headers {
                write!(stream, "{name}: {value}\r\n").unwrap();
            }
            stream.write_all(b"\r\n").unwrap();
            if let Err(error) = stream.write_all(&response.body) {
                // Response limits intentionally close the connection before EOF.
                assert!(
                    matches!(
                        error.kind(),
                        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset
                    ),
                    "fixture response write failed: {error}"
                );
            }
        }
        requests
    });
    (format!("elasticsearch://alice:secret@{address}"), handle)
}

fn identity() -> Value {
    json!({"cluster_name":"test", "version":{"number":"8.17.0"}})
}

#[tokio::test]
async fn native_requests_preserve_auth_body_and_json_results() {
    let (url, server) = server(vec![
        (200, identity()),
        (
            200,
            json!({"hits":{"total":{"value":1,"relation":"eq"},"hits":[{"_index":"books","_id":"1","_source":{"title":"Rust","nested":{"active":true}}}]},"aggregations":{"total":{"value":1}}}),
        ),
    ]);
    let imported = import_connection_url(&url, None).unwrap();
    let connection =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let result = connection
        .execute("POST /books/_search\n{\"query\":{\"match_all\":{}}}")
        .await
        .unwrap();
    assert_eq!(result.result_sets[0].rows.len(), 1);
    assert_eq!(result.result_sets[0].rows[0][1].clipboard_text(), "1");
    assert!(
        result.result_sets[0].rows[0][3]
            .clipboard_text()
            .contains("nested")
    );
    assert!(
        result.result_sets[1].rows[0][0]
            .clipboard_text()
            .contains("aggregations")
    );
    assert!(!connection.capabilities().manual_transactions);
    connection.close().await;
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET / HTTP/1.1"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("authorization: basic ywxpy2u6c2vjcmv0")
    );
    assert!(requests[1].starts_with("POST /books/_search HTTP/1.1"));
    assert!(requests[1].contains("{\"query\":{\"match_all\":{}}}"));
    assert!(!requests[1].contains("secret@"));
}

#[tokio::test]
async fn api_key_requests_use_encoded_sensitive_header_and_never_basic_auth() {
    let (url, server) = server(vec![(200, identity()), (200, json!({"status":"green"}))]);
    let mut imported = import_connection_url(&url, None).unwrap();
    imported.profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: ElasticsearchAuth::ApiKey,
        ca_cert_path: None,
    });
    // This is the API's already encoded value, not an id:key pair to encode again.
    let encoded = "dGVzdC1pZDp0ZXN0LXNlY3JldA==";
    let secret = secrecy::SecretString::from(encoded);
    let connection = DatabaseConnection::connect(&imported.profile, Some(&secret))
        .await
        .unwrap();
    let debug = format!("{connection:?}");
    assert!(!debug.contains(encoded));
    assert!(!debug.contains("test-secret"));
    assert!(!debug.contains("secret@"));
    connection.execute("GET /_cluster/health").await.unwrap();
    connection.close().await;
    let requests = server.join().unwrap();
    for request in requests {
        let headers: Vec<_> = request
            .lines()
            .filter(|line| line.to_ascii_lowercase().starts_with("authorization:"))
            .collect();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0], format!("authorization: ApiKey {encoded}"));
        assert!(
            !request
                .to_ascii_lowercase()
                .contains("authorization: basic")
        );
    }
}

#[tokio::test]
async fn invalid_api_key_credentials_fail_before_network_without_exposing_secret() {
    let mut imported = import_connection_url("elasticsearch://alice@127.0.0.1:1", None).unwrap();
    imported.profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: ElasticsearchAuth::ApiKey,
        ca_cert_path: None,
    });
    for value in [
        None,
        Some(""),
        Some("   "),
        Some("secret-key\r\ninjected: value"),
        Some("secret-key\0"),
    ] {
        let secret = value.map(secrecy::SecretString::from);
        let error = DatabaseConnection::connect(&imported.profile, secret.as_ref())
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert!(error.message.to_ascii_lowercase().contains("api key"));
        assert!(!format!("{error:?}").contains("secret-key"));
    }
}

#[tokio::test]
async fn custom_ca_configuration_requires_https_readable_valid_pem_and_strong_tls() {
    let mut imported = import_connection_url("elasticsearch://127.0.0.1:1", None).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ca.pem");
    imported.profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: ElasticsearchAuth::Basic,
        ca_cert_path: Some(path.clone()),
    });
    let error = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert!(error.message.contains("requires VerifyFull HTTPS"));
    imported.profile.ssl_mode = SslMode::VerifyFull;
    let error = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert!(error.message.contains("exists and is readable"));
    for invalid in [
        "",
        "private-file-content-not-a-certificate",
        "-----BEGIN CERTIFICATE-----\n%%%\n-----END CERTIFICATE-----",
    ] {
        std::fs::write(&path, invalid).unwrap();
        let error = DatabaseConnection::connect(&imported.profile, None)
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert!(error.message.contains("PEM"));
        assert!(!error.message.contains("private-file-content"));
        assert!(!error.message.contains("%%%"));
    }
    std::fs::write(
        &path,
        "-----BEGIN CERTIFICATE-----\nbm90LWEtY2VydGlmaWNhdGU=\n-----END CERTIFICATE-----",
    )
    .unwrap();
    let error = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert!(error.message.contains("CA certificate"));
    assert!(!error.message.contains("bm90LWEtY2VydGlmaWNhdGU="));
    imported.profile.elasticsearch_options = None;
    for mode in [SslMode::Prefer, SslMode::Require, SslMode::VerifyCa] {
        imported.profile.ssl_mode = mode;
        let error = DatabaseConnection::connect(&imported.profile, None)
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert!(error.message.contains("TLS mode must be"));
    }
}

#[tokio::test]
async fn http_failures_have_actionable_categories() {
    for (status, category) in [
        (401, ErrorCategory::Authentication),
        (403, ErrorCategory::Permission),
        (409, ErrorCategory::Constraint),
        (429, ErrorCategory::Network),
    ] {
        let (url, server) = server(vec![(
            status,
            json!({"error":{"type":"test_exception","reason":"denied\u{001b}[31m"}}),
        )]);
        let imported = import_connection_url(&url, None).unwrap();
        let error =
            DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
                .await
                .unwrap_err();
        assert_eq!(error.category, category);
        assert_eq!(error.code.as_deref(), Some("test_exception"));
        assert!(!error.message.contains('\u{001b}'));
        server.join().unwrap();
    }
}

#[tokio::test]
async fn document_previews_use_native_pagination_and_reject_foreign_targets() {
    let hits = (0..11)
        .map(|n| json!({"_index":"books","_id":n.to_string(),"_source":{"n":n}}))
        .collect::<Vec<_>>();
    let (url, server) = server(vec![
        (200, identity()),
        (
            200,
            json!({"hits":{"total":{"value":23,"relation":"eq"},"hits":hits}}),
        ),
    ]);
    let imported = import_connection_url(&url, None).unwrap();
    let connection =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let id = CatalogId::new(
        imported.profile.id,
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    let result = connection
        .preview_relation(
            &id,
            &RelationPreviewOptions::default(),
            PageRequest::first(PageSize::Ten),
        )
        .await
        .unwrap();
    assert_eq!(result.result.result_sets[0].rows.len(), 10);
    assert!(result.pagination.has_next);
    assert_eq!(result.pagination.total, TotalRows::Exact(23));
    let foreign = CatalogId::new(
        uuid::Uuid::new_v4(),
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    assert!(
        connection
            .preview_relation(
                &foreign,
                &Default::default(),
                PageRequest::first(PageSize::Ten)
            )
            .await
            .is_err()
    );
    assert!(
        connection
            .preview_relation(
                &id,
                &Default::default(),
                PageRequest::at(PageSize::Ten, 10000)
            )
            .await
            .is_err()
    );
    connection.close().await;
    let requests = server.join().unwrap();
    let body: Value = serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["size"], 11);
    assert_eq!(body["query"], json!({"match_all":{}}));
}

#[tokio::test]
async fn catalog_pages_and_search_use_native_indices_and_mapping() {
    use lazydb::{
        db::catalog::{CatalogRequest, CatalogRequestKey, CatalogTarget, ObjectGroup},
        identity::ConnectionIdentity,
    };
    let (url, server) = server(vec![
        (200, identity()),
        (
            200,
            json!([{"index":"books"},{"index":"logs"},{"index":".security"}]),
        ),
        (
            200,
            json!({"books":{"mappings":{"properties":{"title":{"type":"text","fields":{"keyword":{"type":"keyword"}}}}}}}),
        ),
    ]);
    let imported = import_connection_url(&url, None).unwrap();
    let connection =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let profile_id = imported.profile.id;
    let request = |target| CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 1,
            },
            catalog_epoch: 1,
            request_id: 1,
            target,
            cursor: None,
        },
        scope: imported.profile.catalog_scope.clone(),
        page_size: 500,
    };
    let page = connection
        .load_catalog_page(&request(CatalogTarget::Objects {
            schema: CatalogId::new(profile_id, CatalogKind::Schema, ["default", "default"]),
            group: ObjectGroup::Tables,
        }))
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].native_kind, "INDEX");
    let children = connection
        .load_catalog_page(&request(CatalogTarget::RelationChildren {
            relation: page.entries[0].id.clone(),
        }))
        .await
        .unwrap();
    assert_eq!(children.entries[1].qualified_name.object, "title.keyword");
    connection.close().await;
    let requests = server.join().unwrap();
    assert!(requests[1].starts_with("GET /_cat/indices?"));
    assert!(requests[2].starts_with("GET /books/_mapping"));
}

#[tokio::test]
async fn read_only_adapter_blocks_writes_before_http_dispatch() {
    let (url, server) = server(vec![(200, identity())]);
    let mut imported = import_connection_url(&url, None).unwrap();
    imported.profile.read_only = true;
    let connection = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    let error = connection.execute("DELETE /books").await.unwrap_err();
    assert_eq!(error.category, ErrorCategory::Permission);
    assert_eq!(error.code.as_deref(), Some("profile_read_only"));
    connection.close().await;
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test]
async fn previews_reject_timed_out_and_failed_shard_responses() {
    for partial in [json!({"timed_out":true}), json!({"_shards":{"failed":1}})] {
        let mut response = json!({
            "timed_out":false,
            "_shards":{"total":2,"successful":2,"failed":0},
            "hits":{"total":{"value":1,"relation":"eq"},"hits":[
                {"_index":"books","_id":"1","_source":{"title":"partial"}}
            ]}
        });
        response
            .as_object_mut()
            .unwrap()
            .extend(partial.as_object().unwrap().clone());
        let (url, server) = server(vec![(200, identity()), (200, response)]);
        let imported = import_connection_url(&url, None).unwrap();
        let connection = DatabaseConnection::connect(&imported.profile, None)
            .await
            .unwrap();
        let id = CatalogId::new(
            imported.profile.id,
            CatalogKind::Table,
            ["default", "default", "books"],
        );
        let error = connection
            .preview_relation(&id, &Default::default(), PageRequest::first(PageSize::Ten))
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration, "{partial}");
        assert!(error.message.contains("partial search response"), "{error}");
        connection.close().await;
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("POST /books/_search HTTP/1.1"));
    }
}

#[tokio::test]
async fn last_page_preview_rejects_partial_count_before_searching() {
    let (url, server) = server(vec![
        (200, identity()),
        (
            200,
            json!({"count":10,"_shards":{"total":2,"successful":1,"failed":1}}),
        ),
    ]);
    let imported = import_connection_url(&url, None).unwrap();
    let connection = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    let relation = CatalogId::new(
        imported.profile.id,
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    let error = connection
        .preview_relation(
            &relation,
            &Default::default(),
            PageRequest::last(PageSize::Ten, 0),
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("count returned a partial response"));
    connection.close().await;
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].starts_with("GET /books/_count HTTP/1.1"));
}

#[tokio::test]
async fn preview_lookahead_stays_inside_the_last_result_window_page() {
    for size in PageSize::ALL {
        let offset = 10_000 - size.as_u64();
        let hits = (offset..10_000)
            .map(|n| json!({"_index":"books","_id":n.to_string(),"_source":{"n":n}}))
            .collect::<Vec<_>>();
        let (url, server) = server(vec![
            (200, identity()),
            (
                200,
                json!({"timed_out":false,"_shards":{"failed":0},
                "hits":{"total":{"value":10_001,"relation":"eq"},"hits":hits}}),
            ),
        ]);
        let imported = import_connection_url(&url, None).unwrap();
        let connection = DatabaseConnection::connect(&imported.profile, None)
            .await
            .unwrap();
        let id = CatalogId::new(
            imported.profile.id,
            CatalogKind::Table,
            ["default", "default", "books"],
        );
        let preview = connection
            .preview_relation(&id, &Default::default(), PageRequest::at(size, offset))
            .await
            .unwrap();
        assert_eq!(preview.result.result_sets.len(), 1);
        let rows = &preview.result.result_sets[0].rows;
        assert_eq!(rows.len(), size.get());
        assert_eq!(
            rows.first().unwrap()[1].clipboard_text(),
            offset.to_string()
        );
        assert_eq!(rows.last().unwrap()[1].clipboard_text(), "9999");
        assert_eq!(preview.result.stats.fetched_row_count, size.get());
        assert_eq!(preview.result.stats.row_count, size.get());
        assert_eq!(preview.pagination.offset, offset);
        assert_eq!(preview.pagination.visible_rows, size.get());
        assert_eq!(preview.pagination.total, TotalRows::Exact(10_001));
        assert!(!preview.pagination.has_next);
        assert_eq!(preview.pagination.next_request(), None);
        connection.close().await;
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("POST /books/_search HTTP/1.1"));
        let body: Value =
            serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["from"], offset);
        assert_eq!(body["size"], size.get());
        assert_eq!(body["sort"], json!(["_doc"]));
        assert_eq!(body["track_total_hits"], true);
    }
}

#[tokio::test]
async fn transport_rejects_redirect_with_location_without_contacting_target() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_address = target.local_addr().unwrap();
    target.set_nonblocking(true).unwrap();
    let (stop, stopped) = std::sync::mpsc::channel();
    let redirect_target = thread::spawn(move || {
        let mut contacted = false;
        loop {
            match target.accept() {
                Ok((mut stream, _)) => {
                    contacted = true;
                    stream.set_nonblocking(false).unwrap();
                    // A followed redirect must finish promptly so the assertion can fail.
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let body = identity().to_string();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => panic!("redirect target accept failed: {error}"),
            }
            match stopped.recv_timeout(Duration::from_millis(1)) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        contacted
    });
    let (url, server) = raw_server(vec![HttpResponse {
        status: 302,
        headers: vec![(
            "Location".into(),
            format!("http://{target_address}/redirected"),
        )],
        body: b"{}".to_vec(),
    }]);
    let imported = import_connection_url(&url, None).unwrap();
    let result =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref()).await;
    stop.send(()).unwrap();
    let contacted = redirect_target.join().unwrap();
    let requests = server.join().unwrap();
    assert!(!contacted, "redirect target received an HTTP connection");
    let error = result.unwrap_err();
    assert_eq!(error.category, ErrorCategory::Sql);
    assert_eq!(error.code.as_deref(), Some("302"));
    assert!(error.message.contains("302"));
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn successful_http_status_with_invalid_json_is_a_configuration_error() {
    for body in ["<html>proxy error</html>", "{\"hits\":"] {
        let (url, server) = raw_server(vec![
            HttpResponse::new(200, identity().to_string()),
            HttpResponse::new(200, body),
        ]);
        let imported = import_connection_url(&url, None).unwrap();
        let connection = DatabaseConnection::connect(&imported.profile, None)
            .await
            .unwrap();
        let error = connection.execute("GET /books/_search").await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert!(error.message.contains("non-JSON response"), "{error}");
        connection.close().await;
        assert_eq!(server.join().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn successful_responses_accept_32_mib_and_reject_one_byte_more() {
    const LIMIT: usize = 32 * 1024 * 1024;
    for length in [LIMIT, LIMIT + 1] {
        // Valid JSON plus whitespace isolates the transport limit from result budgets.
        let mut body = br#"{"status":"green"}"#.to_vec();
        body.resize(length, b' ');
        let (url, server) = raw_server(vec![
            HttpResponse::new(200, identity().to_string()),
            HttpResponse::new(200, body),
        ]);
        let imported = import_connection_url(&url, None).unwrap();
        let connection = DatabaseConnection::connect(&imported.profile, None)
            .await
            .unwrap();
        let result = connection.execute("GET /_cluster/health").await;
        connection.close().await;
        assert_eq!(server.join().unwrap().len(), 2);
        if length == LIMIT {
            let result = result.unwrap();
            assert_eq!(
                result.result_sets[0].rows[0][0].clipboard_text(),
                r#"{"status":"green"}"#
            );
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.category, ErrorCategory::Configuration);
            assert!(error.message.contains("exceeds 32 MiB"), "{error}");
        }
    }
}

#[tokio::test]
async fn error_response_limit_preserves_http_classification() {
    const LIMIT: usize = 16 * 1024;
    for (status, category) in [
        (401, ErrorCategory::Authentication),
        (403, ErrorCategory::Permission),
        (409, ErrorCategory::Constraint),
        (429, ErrorCategory::Network),
    ] {
        for length in [LIMIT, LIMIT + 1] {
            let mut body = json!({"error":{"type":"test_exception","reason":""}});
            let padding = length - body.to_string().len();
            body["error"]["reason"] = Value::String("x".repeat(padding));
            let body = body.to_string();
            assert_eq!(body.len(), length);
            let (url, server) = raw_server(vec![HttpResponse::new(status, body)]);
            let imported = import_connection_url(&url, None).unwrap();
            let error = DatabaseConnection::connect(&imported.profile, None)
                .await
                .unwrap_err();
            assert_eq!(error.category, category, "HTTP {status}, {length} bytes");
            if length == LIMIT {
                assert_eq!(error.code.as_deref(), Some("test_exception"));
                assert!(error.message.contains(&"x".repeat(padding)));
            } else {
                // Truncation splits the JSON; classification must still use HTTP status.
                assert_eq!(error.code, Some(status.to_string()));
                assert!(error.message.contains("Elasticsearch request failed"));
                assert!(error.message.len() < 256);
            }
            assert_eq!(server.join().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn search_write_responses_without_decodable_results_have_unknown_outcomes() {
    for scheme in ["elasticsearch", "opensearch"] {
        for oversized in [false, true] {
            let product_identity = if scheme == "opensearch" {
                json!({"version":{"distribution":"opensearch","number":"2.19.4"}})
            } else {
                identity()
            };
            let response = if oversized {
                vec![b'x'; 33 * 1024 * 1024]
            } else {
                b"invalid JSON".to_vec()
            };
            let (url, server) = raw_server(vec![
                HttpResponse::new(200, product_identity.to_string()),
                HttpResponse::new(200, response),
            ]);
            let url = url.replacen("elasticsearch:", &format!("{scheme}:"), 1);
            let imported = import_connection_url(&url, None).unwrap();
            let connection = DatabaseConnection::connect(
                &imported.profile,
                imported.transient_password.as_ref(),
            )
            .await
            .unwrap();
            let error = connection
                .execute("DELETE /books/_doc/1")
                .await
                .unwrap_err();
            assert_eq!(error.category, ErrorCategory::Configuration);
            assert_eq!(
                error.code.as_deref(),
                Some("search_rest_write_outcome_unknown")
            );
            assert!(error.message.contains("outcome may be unknown"));
            assert!(error.message.contains("not replayed"));
            connection.close().await;
            assert_eq!(server.join().unwrap().len(), 2);
        }
    }
}

#[tokio::test]
async fn opensearch_diagnostics_preserve_product_names_in_paths_and_server_reasons() {
    let mut imported = import_connection_url("opensearchs://localhost", None).unwrap();
    let missing =
        std::env::temp_dir().join(format!("Elasticsearch-ca-{}.pem", uuid::Uuid::new_v4()));
    imported.profile.opensearch_options = Some(lazydb::profile::OpenSearchConnectionOptions {
        ca_cert_path: Some(missing.clone()),
        ..Default::default()
    });
    let error = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap_err();
    assert!(error.message.contains("OpenSearch CA"));
    assert!(error.message.contains(&missing.display().to_string()));
    let (url, server) = raw_server(vec![HttpResponse::new(
        403,
        json!({"error":{"reason":"Elasticsearch-named-resource denied"}}).to_string(),
    )]);
    let imported =
        import_connection_url(&url.replacen("elasticsearch:", "opensearch:", 1), None).unwrap();
    let error =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap_err();
    assert!(error.message.starts_with("OpenSearch HTTP"));
    assert!(
        error
            .message
            .contains("Elasticsearch-named-resource denied")
    );
    server.join().unwrap();
}
