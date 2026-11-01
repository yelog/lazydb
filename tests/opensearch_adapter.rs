use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogTarget, ObjectGroup,
        },
        query::QueryBudget,
    },
    identity::ConnectionIdentity,
    model::{
        pagination::{PageRequest, PageSize, TotalRows},
        relation::RelationPreviewOptions,
    },
    profile::{DatabaseKind, OpenSearchAuth, OpenSearchConnectionOptions, import_connection_url},
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

struct Response {
    status: u16,
    body: Vec<u8>,
    headers: String,
}
impl Response {
    fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            body: value.to_string().into_bytes(),
            headers: String::new(),
        }
    }
}

// Finite, loopback-only HTTP fixture. Every expected response must be consumed;
// the deadline catches accidental missing dispatch without hanging the suite.
fn serve(responses: Vec<Response>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        responses.into_iter().map(|response| {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "OpenSearch fixture missing request");
                        thread::park_timeout(Duration::from_millis(1));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length = headers.lines().find_map(|s| s.strip_prefix("content-length: ")).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                    if bytes.len() >= end + 4 + length { break; }
                }
            }
            write!(stream, "HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n", response.status, response.body.len(), response.headers).unwrap();
            if let Err(e) = stream.write_all(&response.body) {
                assert!(matches!(e.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset));
            }
            String::from_utf8(bytes).unwrap()
        }).collect()
    });
    (format!("opensearch://alice:secret@{address}"), task)
}
fn identity() -> Response {
    Response::json(
        200,
        json!({"version":{"distribution":"opensearch","number":"2.19.4"}}),
    )
}
async fn connect(url: &str) -> DatabaseConnection {
    let p = import_connection_url(url, None).unwrap();
    DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
        .await
        .unwrap()
}
fn body(request: &str) -> Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn identity_requires_distribution_and_version_and_rejects_elasticsearch() {
    for value in [
        json!({"version":{"number":"2.19.4"}}),
        json!({"version":{"distribution":"elasticsearch","number":"8.17.0"}}),
        json!({"version":{"distribution":"opensearch"}}),
        json!({"version":{"distribution":"opensearch","number":""}}),
    ] {
        let (url, task) = serve(vec![Response::json(200, value)]);
        let p = import_connection_url(&url, None).unwrap();
        let error = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert!(error.message.contains("OpenSearch"));
        task.join().unwrap();
    }
    let (url, task) = serve(vec![identity()]);
    let es = url.replacen("opensearch:", "elasticsearch:", 1);
    let p = import_connection_url(&es, None).unwrap();
    assert!(
        DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
            .await
            .unwrap_err()
            .message
            .contains("identity mismatch")
    );
    task.join().unwrap();
}

#[tokio::test]
async fn basic_and_none_auth_have_distinct_headers_and_options() {
    for auth in [OpenSearchAuth::Basic, OpenSearchAuth::None] {
        let (url, task) = serve(vec![
            identity(),
            identity(),
            Response::json(200, json!({"ok":true})),
        ]);
        let mut p = import_connection_url(&url, None).unwrap();
        p.profile.opensearch_options = Some(OpenSearchConnectionOptions {
            auth,
            ca_cert_path: None,
        });
        // An unrelated Elasticsearch API-key configuration must never leak over.
        p.profile.elasticsearch_options = Some(lazydb::profile::ElasticsearchConnectionOptions {
            auth: lazydb::profile::ElasticsearchAuth::ApiKey,
            ca_cert_path: None,
        });
        let connection = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
            .await
            .unwrap();
        assert_eq!(connection.kind(), DatabaseKind::OpenSearch);
        let info = connection.probe().await.unwrap();
        assert_eq!(info.kind, DatabaseKind::OpenSearch);
        assert_eq!(info.version, "2.19.4");
        connection.execute("GET /_cluster/health").await.unwrap();
        connection.close().await;
        for request in task.join().unwrap() {
            let headers = request.to_ascii_lowercase();
            assert_eq!(
                headers.contains("authorization: basic ywxpy2u6c2vjcmv0"),
                auth == OpenSearchAuth::Basic
            );
            assert!(!headers.contains("apikey"));
            assert!(!headers.contains("secret@"));
        }
    }
}

#[tokio::test]
async fn authentication_and_permissions_preserve_categories_and_sanitize_errors() {
    for (status, category) in [
        (401, ErrorCategory::Authentication),
        (403, ErrorCategory::Permission),
    ] {
        let (url, task) = serve(vec![Response::json(
            status,
            json!({"error":{"type":"security_exception", "reason":"denied\u{001b}[31m"}}),
        )]);
        let p = import_connection_url(&url, None).unwrap();
        let error = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
            .await
            .unwrap_err();
        assert_eq!(error.category, category);
        assert!(error.message.contains("OpenSearch"));
        assert!(!error.message.contains('\u{001b}'));
        task.join().unwrap();
    }
}

#[tokio::test]
async fn catalog_filters_indices_pages_mapping_and_preserves_request_owners() {
    let indices = json!([{"index":"z"},{"index":"books"},{"index":".security"},{"index":"books"}]);
    let (url, task) = serve(vec![
        identity(),
        Response::json(200, indices.clone()),
        Response::json(200, indices),
        Response::json(
            200,
            json!({"books":{"mappings":{"properties":{"author":{"type":"nested","properties":{"name":{"type":"text","fields":{"keyword":{"type":"keyword"}}}}}}}}}),
        ),
    ]);
    let p = import_connection_url(&url, None).unwrap();
    let connection = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
        .await
        .unwrap();
    let id = p.profile.id;
    let mut request = CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id: id,
                generation: 7,
            },
            catalog_epoch: 3,
            request_id: 11,
            target: CatalogTarget::Objects {
                schema: CatalogId::new(id, CatalogKind::Schema, ["default", "default"]),
                group: ObjectGroup::Tables,
            },
            cursor: None,
        },
        scope: p.profile.catalog_scope.clone(),
        page_size: 1,
    };
    let first = connection.load_catalog_page(&request).await.unwrap();
    first.validate_for(&request).unwrap();
    assert_eq!(first.entries[0].qualified_name.object, "books");
    request.key.cursor = first.next_cursor;
    let second = connection.load_catalog_page(&request).await.unwrap();
    assert_eq!(second.entries[0].qualified_name.object, "z");
    assert!(second.next_cursor.is_none());
    request.key.cursor = None;
    request.key.target = CatalogTarget::RelationChildren {
        relation: first.entries[0].id.clone(),
    };
    request.page_size = 500;
    let fields = connection.load_catalog_page(&request).await.unwrap();
    assert_eq!(
        fields
            .entries
            .iter()
            .map(|e| e.qualified_name.object.as_str())
            .collect::<Vec<_>>(),
        ["author", "author.name", "author.name.keyword"]
    );
    assert_eq!(fields.entries[0].native_kind, "nested");
    request.key.connection.profile_id = uuid::Uuid::new_v4();
    assert!(connection.load_catalog_page(&request).await.is_err());
    connection.close().await;
    let requests = task.join().unwrap();
    assert!(requests[1].starts_with("GET /_cat/indices?format=json"));
    assert!(requests[3].starts_with("GET /books/_mapping "));
}

#[tokio::test]
async fn preview_preserves_total_metadata_empty_results_and_window_limit() {
    let (url, task) = serve(vec![
        identity(),
        Response::json(
            200,
            json!({"hits":{"total":{"value":12000,"relation":"eq"},"hits":[{"_index":"books","_id":"real-id","_source":{"_id":"source-id","_source":{"nested":[1,2]}}}]}}),
        ),
        Response::json(
            200,
            json!({"hits":{"total":{"value":0,"relation":"eq"},"hits":[]}}),
        ),
        Response::json(200, json!({"count":12000,"_shards":{"failed":0}})),
    ]);
    let p = import_connection_url(&url, None).unwrap();
    let connection = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
        .await
        .unwrap();
    let id = CatalogId::new(
        p.profile.id,
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    let preview = connection
        .preview_relation(
            &id,
            &RelationPreviewOptions::default(),
            PageRequest::at(PageSize::Ten, 9990),
        )
        .await
        .unwrap();
    assert_eq!(preview.pagination.total, TotalRows::Exact(12000));
    assert!(!preview.pagination.has_next);
    let set = &preview.result.result_sets[0];
    assert_eq!(set.rows[0][1].clipboard_text(), "real-id");
    assert!(set.rows[0][3].clipboard_text().contains("nested"));
    assert!(set.columns.iter().any(|c| c.name == "_source._id"));
    let empty = connection
        .preview_relation(&id, &Default::default(), PageRequest::first(PageSize::Ten))
        .await
        .unwrap();
    assert!(empty.result.result_sets[0].rows.is_empty());
    assert_eq!(empty.result.result_sets[0].columns[0].name, "_index");
    assert_eq!(empty.pagination.total, TotalRows::Exact(0));
    let error = connection
        .preview_relation(
            &id,
            &Default::default(),
            PageRequest::last(PageSize::Ten, 12000),
        )
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Unsupported);
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
    let requests = task.join().unwrap();
    assert_eq!(body(&requests[1])["size"], 10);
    assert_eq!(body(&requests[1])["from"], 9990);
}

#[tokio::test]
async fn ndjson_partial_failures_and_shared_budgets() {
    let cases = [
        (
            "POST /_bulk\n{\"index\":{\"_index\":\"books\"}}\n{\"x\":1}",
            json!({"errors":true,"items":[{"index":{"status":409,"error":{"type":"conflict"}}}]}),
            "bulk",
        ),
        (
            "POST /_msearch\n{}\n{}",
            json!({"responses":[{"timed_out":true},{"_shards":{"failed":1}}]}),
            "msearch",
        ),
        (
            "POST /_mget\n{\"docs\":[]}",
            json!({"docs":[{"error":{"type":"missing"}}]}),
            "mget",
        ),
        (
            "POST /books/_update_by_query\n{}",
            json!({"failures":[{"cause":{"type":"conflict"}}]}),
            "failure",
        ),
        (
            "POST /books/_search\n{}",
            json!({"timed_out":true,"hits":{"hits":[]}}),
            "incomplete",
        ),
    ];
    let mut responses = vec![identity()];
    responses.extend(cases.iter().map(|(_, v, _)| Response::json(200, v.clone())));
    let (url, task) = serve(responses);
    let connection = connect(&url).await;
    for (input, _, expected) in &cases {
        let DatabaseConnection::OpenSearch(adapter) = &connection else {
            panic!("OpenSearch adapter expected")
        };
        let result = adapter
            .execute_with_budget(
                input,
                QueryBudget {
                    max_rows: 0,
                    max_bytes: 0,
                },
            )
            .await
            .unwrap();
        assert!(result.stats.truncated);
        let error = result.stats.response_error.as_deref().unwrap();
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("OpenSearch"));
    }
    connection.close().await;
    let requests = task.join().unwrap();
    for request in &requests[1..=2] {
        assert!(request.ends_with('\n'));
        assert!(request.contains("application/x-ndjson"));
    }
    assert!(requests[3].contains("application/json"));
}

#[tokio::test]
async fn readonly_and_unsafe_paths_are_rejected_without_dispatch() {
    let (url, task) = serve(vec![identity()]);
    let mut p = import_connection_url(&url, None).unwrap();
    p.profile.read_only = true;
    let connection = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
        .await
        .unwrap();
    for text in [
        "PUT /books/_doc/1\n{}",
        "GET /_unknown",
        "GET /_search?scroll=1m",
        "GET /books/_doc/1?refresh=true",
        "HEAD /books/_doc/1?%72efresh=true",
    ] {
        assert_eq!(
            connection.execute(text).await.unwrap_err().category,
            ErrorCategory::Permission
        );
    }
    for path in [
        "https://outside.invalid/",
        "//outside.invalid/",
        "/books/../_refresh",
        "/books/%2e%2e/_refresh",
        "/books/%252e%252e/_refresh",
        "/books%2f..%2f_refresh",
        "/books/%5cother",
        "/books/./_search",
    ] {
        assert_eq!(
            connection
                .execute(&format!("GET {path}"))
                .await
                .unwrap_err()
                .category,
            ErrorCategory::Configuration
        );
    }
    connection.close().await;
    assert_eq!(task.join().unwrap().len(), 1);
}

#[tokio::test]
async fn redirect_and_response_limit_are_enforced() {
    let mut redirect = Response::json(302, json!({}));
    redirect.headers = "Location: http://127.0.0.1:1/escaped\r\n".into();
    let (url, task) = serve(vec![
        identity(),
        redirect,
        Response {
            status: 200,
            body: vec![b' '; 32 * 1024 * 1024 + 1],
            headers: String::new(),
        },
    ]);
    let connection = connect(&url).await;
    let error = connection
        .execute("GET /_cluster/health")
        .await
        .unwrap_err();
    assert!(error.message.contains("302"));
    let error = connection
        .execute("GET /_cluster/health")
        .await
        .unwrap_err();
    assert!(error.message.contains("32 MiB"));
    connection.close().await;
    assert_eq!(task.join().unwrap().len(), 3);
}

#[tokio::test]
async fn search_rows_and_metadata_share_row_and_byte_budgets() {
    let search = json!({"hits":{"total":{"value":2,"relation":"eq"},"hits":[{"_id":"1","_source":{"nested":{"n":1}}},{"_id":"2","_source":{"n":2}}]},"aggregations":{"sum":{"value":3}}});
    let (url, task) = serve(vec![
        identity(),
        Response::json(200, search.clone()),
        Response::json(200, search),
    ]);
    let connection = connect(&url).await;
    let DatabaseConnection::OpenSearch(adapter) = &connection else {
        panic!("OpenSearch adapter expected")
    };
    for budget in [
        QueryBudget {
            max_rows: 1,
            max_bytes: 1024 * 1024,
        },
        QueryBudget {
            max_rows: 100,
            max_bytes: 1,
        },
    ] {
        let outcome = adapter
            .execute_with_budget("POST /books/_search\n{}", budget)
            .await
            .unwrap();
        assert!(outcome.stats.truncated);
        assert_eq!(outcome.stats.fetched_row_count, 3);
        assert_eq!(outcome.stats.row_count, usize::from(budget.max_rows == 1));
        assert_eq!(outcome.result_sets.len(), 2);
        assert!(outcome.result_sets[1].rows.is_empty());
    }
    connection.close().await;
    task.join().unwrap();
}

#[tokio::test]
async fn result_window_never_turns_an_inexact_total_into_an_exact_document_count() {
    let hits = (0..10)
        .map(|n| json!({"_id":n.to_string(),"_source":{}}))
        .collect::<Vec<_>>();
    let (url, task) = serve(vec![
        identity(),
        Response::json(
            200,
            json!({"hits":{"total":{"value":12000,"relation":"gte"},"hits":hits.clone()}}),
        ),
        Response::json(200, json!({"hits":{"hits":hits}})),
    ]);
    let p = import_connection_url(&url, None).unwrap();
    let connection = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
        .await
        .unwrap();
    let relation = CatalogId::new(
        p.profile.id,
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    for expected in [12000, 10000] {
        let preview = connection
            .preview_relation(
                &relation,
                &Default::default(),
                PageRequest::at(PageSize::Ten, 9990),
            )
            .await
            .unwrap();
        assert_eq!(preview.pagination.total, TotalRows::LowerBound(expected));
        assert!(!preview.pagination.has_next);
    }
    connection.close().await;
    task.join().unwrap();
}

#[tokio::test]
async fn preview_rejects_expanded_documents_above_shared_result_byte_budget() {
    // Well below the HTTP cap, but _source plus _hit and source columns expand
    // the retained result above the common byte budget.
    let value = json!({"hits":{"total":{"value":1,"relation":"eq"},"hits":[{"_id":"1","_source":{"payload":"x".repeat(6*1024*1024)}}]}});
    let (url, task) = serve(vec![identity(), Response::json(200, value)]);
    let p = import_connection_url(&url, None).unwrap();
    let connection = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
        .await
        .unwrap();
    let relation = CatalogId::new(
        p.profile.id,
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    let error = connection
        .preview_relation(
            &relation,
            &Default::default(),
            PageRequest::first(PageSize::Ten),
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("shared 16 MiB result budget"));
    connection.close().await;
    task.join().unwrap();
}

#[tokio::test]
async fn preview_discards_lookahead_before_column_discovery_and_result_byte_budget() {
    let mut hits = (0..10)
        .map(|n| json!({"_id":n.to_string(),"_source":{"title":"visible"}}))
        .collect::<Vec<_>>();
    // This document alone would exceed the retained-result budget after column
    // expansion, but it is only used to establish whether another page exists.
    hits.push(json!({"_id":"lookahead","_source":{"lookahead_only":"x".repeat(6*1024*1024)}}));
    let (url, task) = serve(vec![
        identity(),
        Response::json(200, json!({"hits":{"hits":hits}})),
    ]);
    let p = import_connection_url(&url, None).unwrap();
    let connection = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
        .await
        .unwrap();
    let relation = CatalogId::new(
        p.profile.id,
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    let preview = connection
        .preview_relation(
            &relation,
            &Default::default(),
            PageRequest::first(PageSize::Ten),
        )
        .await
        .unwrap();
    let set = &preview.result.result_sets[0];
    assert_eq!(set.rows.len(), 10);
    assert!(
        set.columns
            .iter()
            .any(|column| column.name == "_source.title")
    );
    assert!(
        !set.columns
            .iter()
            .any(|column| column.name == "_source.lookahead_only")
    );
    assert_eq!(preview.result.stats.row_count, 10);
    assert_eq!(preview.result.stats.fetched_row_count, 11);
    assert!(!preview.result.stats.truncated);
    assert_eq!(preview.pagination.visible_rows, 10);
    assert_eq!(preview.pagination.total, TotalRows::LowerBound(11));
    assert!(preview.pagination.has_next);
    connection.close().await;
    task.join().unwrap();
}

#[tokio::test]
async fn preview_rejects_timeout_and_shard_failure_including_count() {
    let (url, task) = serve(vec![
        identity(),
        Response::json(200, json!({"timed_out":true,"hits":{"hits":[]}})),
        Response::json(200, json!({"_shards":{"failed":1},"hits":{"hits":[]}})),
        Response::json(200, json!({"count":23,"_shards":{"failed":1}})),
    ]);
    let p = import_connection_url(&url, None).unwrap();
    let connection = DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
        .await
        .unwrap();
    let relation = CatalogId::new(
        p.profile.id,
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    for page in [
        PageRequest::first(PageSize::Ten),
        PageRequest::first(PageSize::Ten),
        PageRequest::last(PageSize::Ten, 23),
    ] {
        assert!(
            connection
                .preview_relation(&relation, &Default::default(), page)
                .await
                .unwrap_err()
                .message
                .contains("partial")
        );
    }
    connection.close().await;
    task.join().unwrap();
}

#[tokio::test]
async fn malformed_profile_hosts_are_rejected_before_network() {
    let mut p = import_connection_url("opensearch://localhost", None).unwrap();
    for host in [
        "user@other.invalid",
        "other.invalid/path",
        "other.invalid?query",
        "other.invalid#fragment",
    ] {
        p.profile.host = Some(host.into());
        assert_eq!(
            DatabaseConnection::connect(&p.profile, p.transient_password.as_ref())
                .await
                .unwrap_err()
                .category,
            ErrorCategory::Configuration
        );
    }
}

#[tokio::test]
async fn cancellation_stops_local_waiting_for_an_inflight_http_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let mut accepted_tx = Some(accepted_tx);
        for number in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 1024];
            while !bytes.windows(4).any(|s| s == b"\r\n\r\n") {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            if number == 0 {
                let body = identity().body;
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            } else {
                accepted_tx.take().unwrap().send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                // Releasing the fixture does not claim server-side cancellation.
            }
        }
    });
    let connection = connect(&format!("opensearch://{address}")).await;
    let query = tokio::spawn(async move { connection.execute("GET /_cluster/health").await });
    tokio::time::timeout(Duration::from_secs(5), accepted_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(!query.is_finished());
    query.abort();
    let cancelled = tokio::time::timeout(Duration::from_secs(1), query)
        .await
        .unwrap()
        .unwrap_err();
    assert!(cancelled.is_cancelled());
    release_tx.send(()).unwrap();
    server.join().unwrap();
}

#[tokio::test]
async fn failed_write_network_outcome_is_explicit_and_never_replayed() {
    let (url, task) = serve(vec![identity()]);
    let connection = connect(&url).await;
    task.join().unwrap(); // The listener is now gone; no second dispatch can succeed.
    let error = connection
        .execute("PUT /books/_doc/1\n{}")
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Network);
    assert!(error.message.contains("outcome may be unknown"));
    assert!(error.message.contains("not replayed"));
    assert!(!error.message.contains("secret"));
    connection.close().await;
}
