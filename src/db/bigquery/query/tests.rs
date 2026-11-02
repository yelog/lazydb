use super::*;
use serde_json::json;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug)]
struct Request {
    method: String,
    url: url::Url,
    body: Value,
    content_length: Option<usize>,
}

impl Request {
    fn reference(&self) -> Value {
        if self.method == "POST" && !self.url.path().ends_with("/cancel") {
            return self.body["jobReference"].clone();
        }
        let parts = self.url.path_segments().unwrap().collect::<Vec<_>>();
        let id = if parts.last() == Some(&"cancel") {
            parts[parts.len() - 2]
        } else {
            parts[parts.len() - 1]
        };
        let location = self
            .url
            .query_pairs()
            .find(|(key, _)| key == "location")
            .map(|(_, value)| value.into_owned());
        json!({"projectId": "billing-project", "jobId": id, "location": location})
    }
}

enum Reply {
    Json(u16, Value),
    Close,
    Malformed,
    Oversized,
}

struct Server {
    endpoint: url::Url,
    requests: Arc<Mutex<Vec<Request>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn new(handler: impl Fn(&Request, usize) -> Reply + Send + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = url::Url::parse(&format!(
            "http://{}/bigquery/v2/",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let offset = loop {
                    let mut chunk = [0; 2048];
                    let size = stream.read(&mut chunk).await.unwrap();
                    if size == 0 {
                        break None;
                    }
                    bytes.extend_from_slice(&chunk[..size]);
                    assert!(bytes.len() < 64 * 1024);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break Some(end + 4);
                    }
                };
                let Some(offset) = offset else {
                    continue;
                };
                let headers = std::str::from_utf8(&bytes[..offset]).unwrap();
                let mut first = headers.lines().next().unwrap().split_whitespace();
                let method = first.next().unwrap().to_owned();
                let url =
                    url::Url::parse(&format!("http://fixture{}", first.next().unwrap())).unwrap();
                let content_length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse::<usize>().unwrap());
                let length = content_length.unwrap_or(0);
                assert!(headers.lines().any(|line| {
                    line.eq_ignore_ascii_case("authorization: Bearer fixture-access-token")
                }));
                while bytes.len() < offset + length {
                    let mut chunk = [0; 2048];
                    let size = stream.read(&mut chunk).await.unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&chunk[..size]);
                }
                let body = if length == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[offset..offset + length]).unwrap()
                };
                let request = Request {
                    method,
                    url,
                    body,
                    content_length,
                };
                let index = {
                    let mut requests = recorded.lock().unwrap();
                    let index = requests.len();
                    requests.push(request.clone());
                    index
                };
                let reply = handler(&request, index);
                let wire = match reply {
                    Reply::Close => continue,
                    Reply::Malformed => {
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{".to_owned()
                    }
                    Reply::Oversized => {
                        "HTTP/1.1 200 OK\r\nContent-Length: 16777217\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    }
                    Reply::Json(status, value) => {
                        let body = value.to_string();
                        format!(
                            "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    }
                };
                // Cancellation may close a socket while the fixture replies.
                let _ = stream.write_all(wire.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }

    fn adapter(&self) -> BigQueryAdapter {
        BigQueryAdapter::fixture(self.endpoint.clone())
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn job(request: &Request, state: &str) -> Value {
    json!({
        "jobReference": request.reference(),
        "status": {"state": state},
        "statistics": {"query": {
            "totalBytesProcessed": "12", "totalBytesBilled": "24", "totalSlotMs": "5", "cacheHit": false
        }}
    })
}

fn page(request: &Request, values: &[&str], total: usize, next: Option<&str>) -> Value {
    let mut result = json!({
        "jobReference": request.reference(), "jobComplete": true,
        "schema": {"fields": [{"name": "answer", "type": "INTEGER"}]},
        "totalRows": total.to_string(),
        "rows": values.iter().map(|value| json!({"f": [{"v": value}]})).collect::<Vec<_>>()
    });
    if let Some(next) = next {
        result["pageToken"] = json!(next);
    }
    result
}

fn assert_identity(requests: &[Request]) {
    let reference = &requests[0].body["jobReference"];
    let id = reference["jobId"].as_str().unwrap();
    assert!(id.starts_with("lazydb_"));
    assert_eq!(reference["projectId"], "billing-project");
    assert_eq!(reference["location"], "US");
    for request in &requests[1..] {
        assert!(request.url.path().contains("/projects/billing-project/"));
        assert!(request.url.path().contains(id));
        assert!(
            request
                .url
                .query_pairs()
                .any(|(key, value)| key == "location" && value == "US")
        );
    }
}

#[tokio::test]
async fn immediate_job_preserves_configuration_identity_and_statistics() {
    let server = Server::new(|request, index| {
        Reply::Json(
            200,
            if index == 0 {
                job(request, "DONE")
            } else {
                page(request, &["42"], 1, None)
            },
        )
    })
    .await;
    let result = server
        .adapter()
        .execute("SELECT 42 AS answer")
        .await
        .unwrap();
    assert_eq!(
        result.result_sets[0].rows,
        vec![vec![CellValue::Integer(42)]]
    );
    let stats = result.stats.bigquery.unwrap();
    assert_eq!(stats.project, "billing-project");
    assert_eq!(stats.location.as_deref(), Some("US"));
    assert_eq!(stats.total_bytes_processed, Some(12));
    assert_eq!(stats.total_bytes_billed, Some(24));
    assert_eq!(stats.slot_millis, Some(5));
    assert_eq!(stats.cache_hit, Some(false));
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_identity(&requests);
    let query = &requests[0].body["configuration"]["query"];
    assert_eq!(query["useLegacySql"], false);
    assert_eq!(query["maximumBytesBilled"], "1000000000");
    assert_eq!(
        query["defaultDataset"],
        json!({"projectId":"data-project", "datasetId":"fixture"})
    );
    assert!(
        requests[1]
            .url
            .query_pairs()
            .any(|(key, value)| key == "timeoutMs" && value == "0")
    );
}

#[tokio::test]
async fn asynchronous_job_polls_and_fetches_nonempty_and_empty_pages() {
    let server = Server::new(|request, index| {
        Reply::Json(
            200,
            match index {
                0 => job(request, "PENDING"),
                1 => job(request, "RUNNING"),
                2 => job(request, "DONE"),
                3 => page(request, &["1"], 2, Some("first")),
                4 => page(request, &[], 2, Some("second")),
                _ => page(request, &["2"], 2, None),
            },
        )
    })
    .await;
    let result = server
        .adapter()
        .execute("SELECT 1 AS answer")
        .await
        .unwrap();
    assert_eq!(
        result.result_sets[0].rows,
        vec![vec![CellValue::Integer(1)], vec![CellValue::Integer(2)]]
    );
    assert!(!result.stats.truncated);
    assert!(result.stats.execution >= Duration::from_millis(300));
    let requests = server.requests();
    assert_eq!(requests.len(), 6);
    assert_identity(&requests);
    assert!(
        requests[4]
            .url
            .query_pairs()
            .any(|(key, value)| key == "pageToken" && value == "first")
    );
    assert!(
        requests[5]
            .url
            .query_pairs()
            .any(|(key, value)| key == "pageToken" && value == "second")
    );
}

#[tokio::test]
async fn result_page_retries_transient_get_without_replaying_sql() {
    let server = Server::new(|request, index| match index {
        0 => Reply::Json(200, job(request, "DONE")),
        1 => Reply::Json(503, json!({"error":{"message":"temporarily unavailable"}})),
        _ => Reply::Json(200, page(request, &["42"], 1, None)),
    })
    .await;
    let outcome = server.adapter().execute("SELECT 42").await.unwrap();
    assert_eq!(
        outcome.result_sets[0].rows,
        vec![vec![CellValue::Integer(42)]]
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].url, requests[2].url);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    assert_identity(&requests);
}

#[tokio::test]
async fn row_and_byte_budgets_truncate_without_fetching_another_page() {
    for budget in [
        QueryBudget {
            max_rows: 1,
            max_bytes: 1000,
        },
        QueryBudget {
            max_rows: 100,
            max_bytes: 0,
        },
    ] {
        let server = Server::new(|request, index| {
            Reply::Json(
                200,
                if index == 0 {
                    job(request, "DONE")
                } else {
                    page(request, &["1"], 2, Some("must-not-fetch"))
                },
            )
        })
        .await;
        let result = server
            .adapter()
            .execute_with_budget("SELECT 1 AS answer", budget)
            .await
            .unwrap();
        assert!(result.stats.truncated);
        assert_eq!(
            result.result_sets[0].rows.len(),
            usize::from(budget.max_rows == 1)
        );
        assert_eq!(server.requests().len(), 2);
    }
}

#[tokio::test]
async fn uncertain_submission_recovers_original_job_without_replaying_dml() {
    for lost in [true, false] {
        let server = Server::new(move |request, index| match index {
            0 if lost => Reply::Close,
            0 => Reply::Json(503, json!({"error":{"message":"unavailable"}})),
            1 => {
                let mut value = job(request, "DONE");
                value["statistics"]["query"]["numDmlAffectedRows"] = json!("3");
                Reply::Json(200, value)
            }
            _ => Reply::Json(
                200,
                json!({"jobReference":request.reference(), "jobComplete":true}),
            ),
        })
        .await;
        let result = server
            .adapter()
            .execute("DELETE FROM fixture.table WHERE TRUE")
            .await
            .unwrap();
        assert_eq!(result.result_sets[0].affected_rows, 3);
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_identity(&requests);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "POST")
                .count(),
            1
        );
        assert!(requests[1].url.path().contains("/jobs/"));
        assert_eq!(requests[1].method, "GET");
    }
}

#[tokio::test]
async fn malformed_and_oversized_submission_responses_recover_original_dml_job() {
    for malformed in [true, false] {
        let server = Server::new(move |request, index| match index {
            0 if malformed => Reply::Malformed,
            0 => Reply::Oversized,
            1 => {
                let mut value = job(request, "DONE");
                value["statistics"]["query"]["numDmlAffectedRows"] = json!("3");
                Reply::Json(200, value)
            }
            _ => Reply::Json(
                200,
                json!({"jobReference":request.reference(), "jobComplete":true}),
            ),
        })
        .await;
        let outcome = server
            .adapter()
            .execute("DELETE FROM fixture.table WHERE TRUE")
            .await
            .expect("an unusable insertion response must recover the original job");
        assert_eq!(outcome.result_sets[0].affected_rows, 3);
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_identity(&requests);
        assert_eq!(requests[1].method, "GET");
        assert!(requests[1].url.path().contains("/jobs/"));
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "POST")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn unrecoverable_submission_reports_unknown_outcome() {
    let server = Server::new(|_, index| {
        if index == 0 {
            Reply::Close
        } else {
            Reply::Json(404, json!({"error":{"message":"not found"}}))
        }
    })
    .await;
    let error = server
        .adapter()
        .execute("DELETE FROM fixture.table WHERE TRUE")
        .await
        .unwrap_err();
    assert_eq!(error.code.as_deref(), Some("job_outcome_unknown"));
    assert!(error.message.contains("SQL was not resubmitted"));
    assert!(error.message.contains("remote final state unknown"));
    let requests = server.requests();
    assert_eq!(requests.len(), 4);
    assert!(requests[2].url.path().ends_with("/cancel"));
    assert_identity(&requests);
}

#[tokio::test]
async fn http_errors_map_categories_and_redact_credentials() {
    for (status, category) in [
        (401, ErrorCategory::Authentication),
        (403, ErrorCategory::Permission),
        (429, ErrorCategory::Network),
        (500, ErrorCategory::Network),
        (503, ErrorCategory::Network),
    ] {
        let server = Server::new(move |_, _| {
            Reply::Json(
                status,
                json!({"error":{"message":"Bearer secret-server-token"}}),
            )
        })
        .await;
        let error = server
            .adapter()
            .request(
                Method::GET,
                &["projects", "billing-project", "datasets"],
                &[],
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.category, category);
        assert_eq!(error.code.as_deref(), Some(status.to_string().as_str()));
        assert!(!format!("{error:?}").contains("secret-server-token"));
    }
}

#[tokio::test]
async fn oversized_response_is_rejected_before_body_allocation() {
    let server = Server::new(|_, _| Reply::Oversized).await;
    let error = server
        .adapter()
        .request(Method::GET, &["projects"], &[], None)
        .await
        .unwrap_err();
    assert_eq!(error.code.as_deref(), Some("bigquery_response"));
    assert!(error.message.contains("16 MiB"));
}

#[tokio::test]
async fn terminal_job_errors_do_not_fetch_rows() {
    for (reason, category) in [
        ("invalidQuery", ErrorCategory::Sql),
        ("accessDenied", ErrorCategory::Permission),
        ("stopped", ErrorCategory::Cancelled),
    ] {
        let server = Server::new(move |request, _| {
            let mut value = job(request, "DONE");
            value["status"]["errorResult"] =
                json!({"reason":reason, "message":"refresh_token=secret-job-token"});
            Reply::Json(200, value)
        })
        .await;
        let error = server.adapter().execute("SELECT 1").await.unwrap_err();
        assert_eq!(error.category, category);
        assert_eq!(error.code.as_deref(), Some(reason));
        assert!(!format!("{error:?}").contains("secret-job-token"));
        assert_eq!(server.requests().len(), 1);
    }
}

#[tokio::test]
async fn polling_failures_cancel_original_job_and_report_remote_final_state() {
    for status in [403, 503] {
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = stopped.clone();
        let server = Server::new(move |request, index| {
            if index == 0 {
                return Reply::Json(200, job(request, "RUNNING"));
            }
            if request.url.path().ends_with("/cancel") {
                cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                return Reply::Json(200, json!({"job":job(request, "RUNNING")}));
            }
            if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                let mut value = job(request, "DONE");
                value["status"]["errorResult"] = json!({"reason":"stopped"});
                return Reply::Json(200, value);
            }
            Reply::Json(status, json!({"error":{"message":"poll unavailable"}}))
        })
        .await;
        let error = server
            .adapter()
            .execute("DELETE FROM fixture.table WHERE TRUE")
            .await
            .unwrap_err();
        assert!(
            stopped.load(std::sync::atomic::Ordering::SeqCst),
            "poll failure must cancel a potentially running job: {error:?}"
        );
        assert!(
            error.message.contains("remote job confirmed stopped"),
            "{error:?}"
        );
        let requests = server.requests();
        assert_identity(&requests);
        let polling_attempts = if status == 503 { 4 } else { 1 };
        assert_eq!(requests.len(), 1 + polling_attempts + 2);
        assert_eq!(
            requests
                .iter()
                .filter(
                    |request| request.method == "POST" && !request.url.path().ends_with("/cancel")
                )
                .count(),
            1
        );
        assert!(requests[requests.len() - 2].url.path().ends_with("/cancel"));
        assert_eq!(requests.last().unwrap().method, "GET");
    }
}

#[tokio::test]
async fn untrusted_job_error_reasons_are_redacted_and_terminal_safe() {
    for reason in [
        "Bearer secret-reason-token",
        "refresh_token=secret-reason-token",
        "bad\u{1b}[2J\nreason\u{7}",
    ] {
        let server = Server::new(move |request, _| {
            let mut value = job(request, "DONE");
            value["status"]["errorResult"] = json!({"reason":reason, "message":"job failed"});
            Reply::Json(200, value)
        })
        .await;
        let error = server.adapter().execute("SELECT 1").await.unwrap_err();
        let rendered = format!("{error:?}");
        assert!(
            !rendered.contains("secret-reason-token"),
            "job reason leaked through an error field"
        );
        assert!(
            !error
                .message
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        );
        assert!(
            !error
                .code
                .as_deref()
                .unwrap_or_default()
                .chars()
                .any(char::is_control)
        );
        assert_eq!(server.requests().len(), 1);
    }
}

#[tokio::test]
async fn native_cancel_sends_zero_content_length_without_changing_gets() {
    let server = Server::new(|request, _| {
        if request.method == "POST" {
            assert!(request.url.path().ends_with("/cancel"));
            assert_eq!(request.content_length, Some(0));
            assert_eq!(request.body, Value::Null);
            Reply::Json(200, json!({}))
        } else {
            assert_eq!(request.method, "GET");
            assert_eq!(request.content_length, None);
            let mut value = job(request, "DONE");
            value["status"]["errorResult"] = json!({"reason":"stopped"});
            Reply::Json(200, value)
        }
    })
    .await;
    let job = JobIdentity {
        project: "billing-project".into(),
        id: "known-cancel-job".into(),
        location: Some("US".into()),
    };
    let cancellation = server.adapter().cancel_job(&job, "test cancellation").await;
    assert_eq!(cancellation.code.as_deref(), Some("cancel_requested"));
    assert!(
        cancellation
            .message
            .contains("remote job confirmed stopped")
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.reference()["jobId"] == "known-cancel-job")
    );
}

#[tokio::test]
async fn read_only_writes_are_rejected_without_http_dispatch() {
    let server = Server::new(|_, _| panic!("read-only writes must not reach HTTP")).await;
    let mut adapter = server.adapter();
    adapter.read_only = true;
    for sql in [
        "DELETE FROM fixture.table WHERE TRUE",
        "INSERT INTO fixture.table VALUES (1)",
        "DROP TABLE fixture.table",
    ] {
        let error = adapter.execute(sql).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Permission);
        assert_eq!(error.code.as_deref(), Some("profile_read_only"));
    }
    assert!(server.requests().is_empty());
    assert!(authorize_statement("SELECT 1", true).is_ok());
}

#[tokio::test]
async fn polling_failure_preserves_unknown_outcome_and_submission_rejection_keeps_code() {
    for submitted in [false, true] {
        let server = Server::new(move |request, index| {
            if index == 0 && submitted {
                Reply::Json(200, job(request, "RUNNING"))
            } else {
                Reply::Json(403, json!({"error":{"message":"access denied"}}))
            }
        })
        .await;
        let error = server
            .adapter()
            .execute("DELETE FROM fixture.table WHERE TRUE")
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Permission);
        assert_eq!(
            error.code.as_deref(),
            Some(if submitted {
                "job_outcome_unknown"
            } else {
                "403"
            })
        );
        assert_eq!(
            error.message.contains("remote final state unknown"),
            submitted
        );
        if !submitted {
            assert_eq!(server.requests().len(), 1);
            assert!(!error.message.contains("cancellation"));
        }
    }
}

#[tokio::test]
async fn pre_cancelled_request_never_submits_sql() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let server = Server::new(|request, _| {
        if request.url.path().ends_with("/cancel") {
            return Reply::Json(200, json!({}));
        }
        let mut value = job(request, "DONE");
        value["status"]["errorResult"] = json!({"reason":"stopped"});
        Reply::Json(200, value)
    })
    .await;
    let error = server
        .adapter()
        .execute_cancellable_with_budget(
            "DELETE FROM fixture.table WHERE TRUE",
            QueryBudget::UNBOUNDED,
            cancel,
        )
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Cancelled);
    assert!(
        server
            .requests()
            .iter()
            .all(|request| request.method != "POST" || request.url.path().ends_with("/cancel"))
    );
}

#[tokio::test]
async fn deadline_and_explicit_cancellation_cancel_original_remote_job() {
    for explicit in [false, true] {
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        let server = Server::new(move |request, index| {
            if index == 0 {
                if explicit {
                    trigger.cancel();
                }
                return Reply::Json(200, job(request, "RUNNING"));
            }
            if request.url.path().ends_with("/cancel") {
                return Reply::Json(200, json!({"job":job(request, "RUNNING")}));
            }
            let mut value = job(request, "DONE");
            value["status"]["errorResult"] = json!({"reason":"stopped", "message":"cancelled"});
            Reply::Json(200, value)
        })
        .await;
        let mut adapter = server.adapter();
        adapter.timeout = Duration::from_millis(20);
        let error = adapter
            .execute_cancellable_with_budget("SELECT 1", QueryBudget::UNBOUNDED, cancel)
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Cancelled);
        assert_eq!(error.code.as_deref(), Some("cancel_requested"));
        assert!(error.message.contains("remote job confirmed stopped"));
        assert!(error.message.contains(if explicit {
            "cancelled by caller"
        } else {
            "query deadline exceeded"
        }));
        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].method, "POST");
        assert!(requests[1].url.path().ends_with("/cancel"));
        assert_identity(&requests);
    }
}

#[tokio::test]
async fn changed_identity_and_repeated_page_tokens_are_rejected() {
    for changed_identity in [true, false] {
        let server = Server::new(move |request, index| {
            let mut value = if index == 0 {
                job(request, "DONE")
            } else {
                page(request, &[], 1, Some("repeated"))
            };
            if changed_identity {
                value["jobReference"]["projectId"] = json!("different-project");
            }
            Reply::Json(200, value)
        })
        .await;
        let error = server.adapter().execute("SELECT 1").await.unwrap_err();
        assert!(error.message.contains(if changed_identity {
            "different job identity"
        } else {
            "page token did not advance"
        }));
    }
}

#[test]
fn epoch_parsing_is_exact_for_decimal_scientific_and_negative_timestamps() {
    for (text, expected) in [
        ("0", (0, 0)),
        ("1.123456789", (1, 123456789)),
        ("-0.000001", (-1, 999999000)),
        ("-1.5", (-2, 500000000)),
        ("1.23456789e3", (1234, 567890000)),
        ("1E-9", (0, 1)),
        ("-1e-9", (-1, 999999999)),
    ] {
        assert_eq!(epoch_parts(text), Some(expected), "{text}");
    }
    for text in [
        "NaN",
        "Infinity",
        "1e9999999999",
        "1.0000000001",
        "--1",
        "1e",
        "1.2.3",
    ] {
        assert_eq!(epoch_parts(text), None, "{text}");
    }
}

fn field(kind: &str) -> Field {
    Field {
        name: "value".into(),
        kind: kind.into(),
        mode: "NULLABLE".into(),
        fields: Vec::new(),
    }
}

#[test]
fn numeric_bytes_and_temporal_values_preserve_native_types() {
    for kind in ["NUMERIC", "BIGNUMERIC"] {
        let value = "12345678901234567890.123456789012345678";
        assert_eq!(
            decode(&json!(value), &field(kind)).unwrap(),
            CellValue::Text(value.into())
        );
    }
    assert_eq!(
        decode(&json!("9223372036854775807"), &field("INT64")).unwrap(),
        CellValue::Integer(i64::MAX)
    );
    assert!(decode(&json!("9223372036854775808"), &field("INT64")).is_err());
    assert_eq!(
        decode(&json!("AAEC/w=="), &field("BYTES")).unwrap(),
        CellValue::Bytes(vec![0, 1, 2, 255])
    );
    assert!(matches!(
        decode(&json!("2026-11-01"), &field("DATE")).unwrap(),
        CellValue::Date(_)
    ));
    assert!(matches!(
        decode(&json!("12:34:56.123456"), &field("TIME")).unwrap(),
        CellValue::Time(_)
    ));
    assert!(matches!(
        decode(&json!("2026-11-01 12:34:56.123456"), &field("DATETIME")).unwrap(),
        CellValue::DateTime(_)
    ));
    let CellValue::Timestamp(timestamp) = decode(&json!("-0.000001"), &field("TIMESTAMP")).unwrap()
    else {
        panic!("timestamp expected")
    };
    assert_eq!(timestamp.timestamp(), -1);
    assert_eq!(timestamp.timestamp_subsec_nanos(), 999999000);
    assert_eq!(
        decode(&Value::Null, &field("STRING")).unwrap(),
        CellValue::Null
    );
    assert!(matches!(
        decode(&json!("POINT(0 0)"), &field("GEOGRAPHY")).unwrap(),
        CellValue::Unsupported { .. }
    ));
}

#[test]
fn nested_and_repeated_values_preserve_nulls_and_decimal_strings() {
    let mut decimal = field("BIGNUMERIC");
    decimal.name = "amount".into();
    let mut repeated = field("INT64");
    repeated.name = "numbers".into();
    repeated.mode = "REPEATED".into();
    let record = Field {
        name: "record".into(),
        kind: "RECORD".into(),
        mode: "NULLABLE".into(),
        fields: vec![decimal, repeated],
    };
    let input = json!({"f":[{"v":"999999999999999999.000000001"},{"v":[{"v":"1"},{"v":null}]}]});
    let CellValue::Text(text) = decode(&input, &record).unwrap() else {
        panic!("nested JSON expected")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"amount":"999999999999999999.000000001", "numbers":["1",null]})
    );
    assert!(native_type(&record).contains("ARRAY<INT64>"));
    assert!(decode(&json!({"f":[]}), &record).is_err());
}

#[test]
fn anonymous_and_duplicate_struct_fields_preserve_each_value_in_order() {
    for names in [["", ""], ["duplicate", "duplicate"], ["named", ""]] {
        let fields = names.map(|name| Field {
            name: name.to_owned(),
            ..field("INT64")
        });
        let record = Field {
            name: "record".into(),
            kind: "RECORD".into(),
            mode: "NULLABLE".into(),
            fields: fields.to_vec(),
        };
        let input = json!({"f":[{"v":"1"},{"v":"2"}]});
        let CellValue::Text(text) = decode(&input, &record).unwrap() else {
            panic!("nested JSON expected")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!([
                {"name":names[0], "value":"1"},
                {"name":names[1], "value":"2"}
            ]),
            "STRUCT field names {names:?} must not overwrite values"
        );
    }
}

#[test]
fn repeated_structs_preserve_duplicate_nested_fields_and_null_elements() {
    let mut child = field("BIGNUMERIC");
    child.name = "duplicate".into();
    let record = Field {
        name: "records".into(),
        kind: "RECORD".into(),
        mode: "REPEATED".into(),
        fields: vec![child.clone(), child],
    };
    let input = json!([
        {"v":{"f":[{"v":"12345678901234567890.000000001"},{"v":"2.5"}]}},
        {"v":{"f":[{"v":null},{"v":"4"}]}},
        {"v":null}
    ]);
    let CellValue::Text(text) = decode(&input, &record).unwrap() else {
        panic!("repeated nested JSON expected")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!([
            [
                {"name":"duplicate", "value":"12345678901234567890.000000001"},
                {"name":"duplicate", "value":"2.5"}
            ],
            [
                {"name":"duplicate", "value":null},
                {"name":"duplicate", "value":"4"}
            ],
            null
        ])
    );
}

#[test]
fn scripts_and_manual_transactions_are_rejected_before_submission() {
    for sql in [
        "SELECT 1; SELECT 2",
        "BEGIN TRANSACTION",
        "COMMIT",
        "ROLLBACK",
        "SET x = 1",
        "CALL fixture.proc()",
    ] {
        assert_eq!(
            authorize_statement(sql, false).unwrap_err().category,
            ErrorCategory::Unsupported,
            "{sql}"
        );
    }
    assert!(authorize_statement("SELECT 1", false).is_ok());
    assert!(authorize_statement("DELETE FROM fixture.table WHERE TRUE", false).is_ok());
}
