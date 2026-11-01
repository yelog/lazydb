pub(super) mod catalog;
mod relation;

use std::{collections::BTreeSet, time::Duration};

use futures_util::StreamExt;
use reqwest::{Client, Method, StatusCode, Url, header};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use crate::{
    db::{
        DatabaseError, ErrorCategory, ServerInfo,
        query::{ColumnMeta, QueryBudget, QueryOutcome, QueryOutcomeAccumulator},
        value::CellValue,
    },
    profile::{ConnectionProfile, DatabaseKind, ElasticsearchAuth, OpenSearchAuth, SslMode},
    security::sanitize_terminal_text,
};

const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct ElasticsearchAdapter {
    client: Client,
    kind: DatabaseKind,
    endpoint: Url,
    user: Option<String>,
    password: Option<SecretString>,
    api_key: Option<header::HeaderValue>,
    read_only: bool,
    pub(crate) profile_id: uuid::Uuid,
    pub(crate) database: String,
}

/// A single Kibana-style REST request. Paths always remain on the configured server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ElasticsearchRequest {
    pub method: Method,
    pub path: String,
    pub body: Option<String>,
}

impl ElasticsearchRequest {
    pub fn parse(input: &str) -> Result<Self, DatabaseError> {
        let input = input.trim();
        let (line, body) = input.split_once('\n').unwrap_or((input, ""));
        let mut words = line.split_whitespace();
        let method = match words.next() {
            Some("GET") => Method::GET,
            Some("HEAD") => Method::HEAD,
            Some("POST") => Method::POST,
            Some("PUT") => Method::PUT,
            Some("PATCH") => Method::PATCH,
            Some("DELETE") => Method::DELETE,
            _ => {
                return Err(DatabaseError::configuration(
                    "Expected METHOD /path followed by an optional JSON body",
                ));
            }
        };
        let path = words.next().ok_or_else(|| {
            DatabaseError::configuration("Elasticsearch request path is required")
        })?;
        if words.next().is_some() {
            return Err(DatabaseError::configuration(
                "Execute one Elasticsearch REST request at a time",
            ));
        }
        validate_path(path)?;
        let body = body.trim();
        let body = if body.is_empty() {
            None
        } else if is_ndjson(path) {
            for line in body.lines().filter(|line| !line.trim().is_empty()) {
                serde_json::from_str::<Value>(line).map_err(|_| {
                    DatabaseError::configuration("Invalid Elasticsearch NDJSON body")
                })?;
            }
            Some(format!("{body}\n"))
        } else {
            serde_json::from_str::<Value>(body).map_err(|error| {
                DatabaseError::configuration(format!("Invalid Elasticsearch JSON body: {error}"))
            })?;
            Some(body.to_owned())
        };
        Ok(Self {
            method,
            path: path.to_owned(),
            body,
        })
    }

    /// Only known read APIs bypass write confirmation. GET alone is not proof
    /// of a read: Elasticsearch also supports stateful APIs such as scroll.
    pub fn is_read_only(&self) -> bool {
        Self::read_only_parts(&self.method, &self.path, self.body.is_some())
    }

    fn read_only_parts(method: &Method, path: &str, has_body: bool) -> bool {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        if url::form_urlencoded::parse(query.as_bytes())
            .any(|(name, _)| matches!(name.as_ref(), "scroll" | "refresh"))
        {
            return false;
        }
        let segments: Vec<_> = path.trim_matches('/').split('/').collect();
        // Classify the original path conservatively before URL normalization.
        if path.contains('%') || segments.iter().any(|part| matches!(*part, "." | "..")) {
            return false;
        }
        if method == Method::HEAD {
            return !has_body
                && (path == "/"
                    || (segments.len() == 1 && !segments[0].starts_with('_'))
                    || (segments.len() == 3 && segments[1] == "_doc")
                    || (segments.len() <= 3 && segments.contains(&"_alias")));
        }
        if method != Method::GET && method != Method::POST {
            return false;
        }
        let last = segments.last().copied().unwrap_or_default();
        if segments.len() <= 2
            && matches!(
                last,
                "_search" | "_count" | "_msearch" | "_mget" | "_field_caps" | "_terms_enum"
            )
        {
            return true;
        }
        if segments.len() == 3 && segments[1] == "_validate" && last == "query" {
            return true;
        }
        if method != Method::GET {
            return false;
        }
        path == "/"
            || (segments.len() <= 2
                && matches!(
                    last,
                    "_mapping"
                        | "_settings"
                        | "_alias"
                        | "_stats"
                        | "_segments"
                        | "_recovery"
                        | "_search_shards"
                ))
            || (segments.first() == Some(&"_cat") && segments.len() >= 2 && segments.len() <= 3)
            || path == "/_cluster/health"
            || path == "/_cluster/stats"
            || path == "/_cluster/state"
            || path == "/_nodes"
            || path.starts_with("/_nodes/")
            || (segments.len() == 3 && segments[1] == "_doc")
    }
}

impl ElasticsearchAdapter {
    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        Self::connect_inner(profile, password).await
    }

    async fn connect_inner(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if !profile.kind.is_search_rest() {
            return Err(DatabaseError::configuration(
                "Search REST adapter requires an Elasticsearch or OpenSearch profile",
            ));
        }
        let product = crate::db::descriptor::descriptor(profile.kind).display_name;
        let host = profile
            .host
            .as_deref()
            .filter(|host| !host.trim().is_empty())
            .ok_or_else(|| DatabaseError::configuration(format!("{product} host is required")))?;
        let port = profile.port.unwrap_or(9200);
        if port == 0 {
            return Err(DatabaseError::configuration(format!(
                "{product} port must be non-zero"
            )));
        }
        let scheme = match profile.ssl_mode {
            SslMode::Disable => "http",
            SslMode::VerifyFull => "https",
            _ => {
                return Err(DatabaseError::configuration(format!(
                    "{product} TLS mode must be Disable (HTTP) or VerifyFull (HTTPS with certificate and hostname verification)"
                )));
            }
        };
        // Authentication belongs to the selected product. Never interpret an
        // Elasticsearch API key as an OpenSearch credential.
        let (basic_auth, api_key_auth, ca_cert_path) = match profile.kind {
            DatabaseKind::OpenSearch => {
                let options = profile.opensearch_options.clone().unwrap_or_default();
                (
                    options.auth == OpenSearchAuth::Basic,
                    false,
                    options.ca_cert_path,
                )
            }
            _ => {
                let options = profile.elasticsearch_options.clone().unwrap_or_default();
                (
                    options.auth == ElasticsearchAuth::Basic,
                    options.auth == ElasticsearchAuth::ApiKey,
                    options.ca_cert_path,
                )
            }
        };
        if ca_cert_path.is_some() && profile.ssl_mode != SslMode::VerifyFull {
            return Err(DatabaseError::configuration(format!(
                "A custom {product} CA certificate requires VerifyFull HTTPS; enable HTTPS or remove the CA path"
            )));
        }
        let api_key = if api_key_auth {
            let secret = password
                .map(ExposeSecret::expose_secret)
                .filter(|secret| !secret.trim().is_empty())
                .ok_or_else(|| {
                    DatabaseError::configuration(
                        "Elasticsearch API Key authentication requires an encoded API key; configure the credential with the encoded value from the create API key response",
                    )
                })?;
            let mut value = header::HeaderValue::from_str(&format!("ApiKey {secret}"))
                .map_err(|_| {
                    DatabaseError::configuration(
                        "Invalid Elasticsearch API key header; configure a single-line encoded API key without control characters",
                    )
                })?;
            value.set_sensitive(true);
            Some(value)
        } else {
            None
        };
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let endpoint = Url::parse(&format!("{scheme}://{host}:{port}/"))
            .map_err(|_| DatabaseError::configuration(format!("Invalid {product} endpoint")))?;
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.path() != "/"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.port_or_known_default() != Some(port)
        {
            return Err(DatabaseError::configuration(
                "Invalid search REST host; enter a hostname or IP address without credentials, path or query",
            ));
        }
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30));
        if let Some(path) = ca_cert_path {
            let pem = std::fs::read(&path).map_err(|error| {
                DatabaseError::configuration(format!(
                    "Could not read {product} CA certificate at {} ({:?}); check that the PEM file exists and is readable",
                    sanitize_terminal_text(&path.display().to_string()), error.kind()
                ))
            })?;
            let certificates = reqwest::Certificate::from_pem_bundle(&pem)
                .ok()
                .filter(|certificates| !certificates.is_empty())
                .ok_or_else(|| {
                    DatabaseError::configuration(format!(
                        "Invalid {product} CA certificate at {}; provide a PEM certificate or PEM certificate bundle",
                        sanitize_terminal_text(&path.display().to_string())
                    ))
                })?;
            // Add explicit trust without replacing the client's default root certificates.
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        }
        let client = builder.build().map_err(|_| {
            DatabaseError::configuration(format!(
                "Could not create {product} HTTP client; check the configured CA certificate contains valid X.509 certificates"
            ))
        })?;
        let adapter = Self {
            client,
            kind: profile.kind,
            endpoint,
            user: basic_auth
                .then(|| profile.user.clone().filter(|user| !user.is_empty()))
                .flatten(),
            password: basic_auth.then(|| password.cloned()).flatten(),
            api_key,
            read_only: profile.read_only,
            profile_id: profile.id,
            database: profile
                .database
                .clone()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "default".into()),
        };
        adapter.probe().await?;
        Ok(adapter)
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let value = self.request_json(Method::GET, "/", None).await?;
        let distribution = value
            .pointer("/version/distribution")
            .and_then(Value::as_str);
        let opensearch = distribution == Some("opensearch");
        if (self.kind == DatabaseKind::OpenSearch && !opensearch)
            || (self.kind == DatabaseKind::Elasticsearch && opensearch)
        {
            return Err(DatabaseError::configuration(format!(
                "{} product identity mismatch: expected {} in GET / version.distribution",
                self.product_name(),
                if self.kind == DatabaseKind::OpenSearch {
                    "opensearch"
                } else {
                    "Elasticsearch, received OpenSearch"
                }
            )));
        }
        let version = value
            .pointer("/version/number")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                DatabaseError::configuration(format!(
                    "{} identity response is missing version.number",
                    self.product_name()
                ))
            })?;
        Ok(ServerInfo {
            kind: self.kind,
            version: sanitize_terminal_text(version),
            database: self.database.clone(),
            current_user: self.user.clone(),
        })
    }

    pub(super) fn product_name(&self) -> &'static str {
        crate::db::descriptor::descriptor(self.kind).display_name
    }

    pub async fn execute_with_budget(
        &self,
        text: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        let request = ElasticsearchRequest::parse(text)?;
        if self.read_only && !request.is_read_only() {
            return Err(DatabaseError {
                category: ErrorCategory::Permission,
                code: Some("profile_read_only".into()),
                message: format!(
                    "This {} connection is read-only; the REST request was not sent",
                    self.product_name()
                ),
                diagnostic: None,
            });
        }
        let started = std::time::Instant::now();
        let value = self
            .request_json(request.method, &request.path, request.body.as_deref())
            .await?;
        let execution = started.elapsed();
        let mut result = outcome(value, budget)?;
        if self.kind == DatabaseKind::OpenSearch {
            result.stats.response_error = result
                .stats
                .response_error
                .map(|error| error.replacen("Elasticsearch", "OpenSearch", 1));
        }
        result.stats.execution = execution;
        Ok(result)
    }

    pub(crate) async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> Result<Value, DatabaseError> {
        self.request_json_inner(method, path, body).await
    }

    async fn request_json_inner(
        &self,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> Result<Value, DatabaseError> {
        validate_path(path)?;
        let potentially_writing =
            !ElasticsearchRequest::read_only_parts(&method, path, body.is_some());
        let uncertain_response = |mut error: DatabaseError| {
            if potentially_writing {
                error.code = Some("search_rest_write_outcome_unknown".into());
                error.message.push_str("; remote write outcome may be unknown. This request was not replayed; inspect remote state before retrying.");
            }
            error
        };
        let network_error = |error| uncertain_response(network_error(error));
        let url = self
            .endpoint
            .join(path)
            .map_err(|_| DatabaseError::configuration("Invalid Elasticsearch request path"))?;
        if url.origin() != self.endpoint.origin() {
            return Err(DatabaseError::configuration(
                "Elasticsearch requests must use the configured endpoint",
            ));
        }
        let head = method == Method::HEAD;
        let mut request = self
            .client
            .request(method, url)
            .header(header::ACCEPT, "application/json");
        if let Some(api_key) = &self.api_key {
            request = request.header(header::AUTHORIZATION, api_key.clone());
        } else if let Some(user) = &self.user {
            request = request.basic_auth(
                user,
                self.password.as_ref().map(ExposeSecret::expose_secret),
            );
        }
        if let Some(body) = body {
            request = request
                .header(
                    header::CONTENT_TYPE,
                    if is_ndjson(path) {
                        "application/x-ndjson"
                    } else {
                        "application/json"
                    },
                )
                .body(body.to_owned());
        }
        let response = request.send().await.map_err(network_error)?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_RESPONSE_BYTES
        } else {
            MAX_ERROR_BYTES
        };
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(network_error)?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                if status.is_success() {
                    return Err(uncertain_response(DatabaseError::configuration(format!(
                        "{} response exceeds 32 MiB; narrow the query or reduce size",
                        self.product_name()
                    ))));
                }
                bytes.extend_from_slice(&chunk[..limit.saturating_sub(bytes.len())]);
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(http_error(self.product_name(), status, &bytes));
        }
        if head || bytes.is_empty() {
            return Ok(json!({"status": status.as_u16()}));
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            uncertain_response(DatabaseError::configuration(format!(
                "{} returned a non-JSON response; use format=json for cat APIs",
                self.product_name()
            )))
        })
    }

    pub async fn close(self) {
        drop(self);
    }
}

fn validate_path(path: &str) -> Result<(), DatabaseError> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains(['\\', '#'])
        || path.chars().any(char::is_control)
    {
        return Err(DatabaseError::configuration(
            "Elasticsearch requests require an absolute /path on the configured server",
        ));
    }
    // Reject traversal before Url::join normalizes dot segments. Decode each
    // path segment separately so encoded separators cannot change its meaning.
    for segment in path.split('?').next().unwrap_or(path).split('/') {
        let decoded = percent_encoding::percent_decode_str(segment)
            .decode_utf8()
            .map_err(|_| DatabaseError::configuration("Invalid UTF-8 in REST request path"))?;
        if matches!(decoded.as_ref(), "." | "..")
            || decoded.contains(['/', '\\', '%'])
            || decoded.chars().any(char::is_control)
        {
            return Err(DatabaseError::configuration(
                "REST request paths must not contain traversal or encoded separators",
            ));
        }
    }
    Ok(())
}

fn is_ndjson(path: &str) -> bool {
    matches!(
        path.split('?').next().unwrap_or(path).rsplit('/').next(),
        Some("_bulk" | "_msearch")
    )
}

pub(crate) fn index_path(index: &str, suffix: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(index.as_bytes()).collect();
    format!("/{}/{suffix}", encoded.replace('+', "%20"))
}

fn network_error(error: reqwest::Error) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Network,
        code: None,
        message: sanitize_terminal_text(&error.without_url().to_string()),
        diagnostic: None,
    }
}

fn http_error(product: &str, status: StatusCode, bytes: &[u8]) -> DatabaseError {
    let value: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let error = &value["error"];
    let reason = error
        .get("reason")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("Elasticsearch request failed");
    DatabaseError {
        category: match status {
            StatusCode::UNAUTHORIZED => ErrorCategory::Authentication,
            StatusCode::FORBIDDEN => ErrorCategory::Permission,
            StatusCode::CONFLICT => ErrorCategory::Constraint,
            StatusCode::TOO_MANY_REQUESTS => ErrorCategory::Network,
            _ => ErrorCategory::Sql,
        },
        code: error
            .get("type")
            .and_then(Value::as_str)
            .map(sanitize_terminal_text)
            .or_else(|| Some(status.as_u16().to_string())),
        message: sanitize_terminal_text(&format!("{product} HTTP {status}: {reason}")),
        diagnostic: None,
    }
}

fn response_error(value: &Value) -> Option<String> {
    if let Some(items) = value.get("items").and_then(Value::as_array) {
        let failed = items
            .iter()
            .filter(|item| {
                item.as_object().is_some_and(|operations| {
                    operations.values().any(|operation| {
                        operation.get("error").is_some_and(|error| !error.is_null())
                            || operation
                                .get("status")
                                .and_then(Value::as_u64)
                                .is_some_and(|status| status >= 300)
                    })
                })
            })
            .count();
        if failed > 0 {
            return Some(format!(
                "Elasticsearch bulk: {failed} of {} item(s) failed. Successful items were not rolled back; inspect response.items before retrying.",
                items.len()
            ));
        }
    }
    if value.get("errors").and_then(Value::as_bool) == Some(true) {
        return Some("Elasticsearch bulk reported errors. Successful items were not rolled back; inspect response.items before retrying.".into());
    }
    if let Some(responses) = value.get("responses").and_then(Value::as_array) {
        let failed = responses
            .iter()
            .filter(|response| {
                response.get("error").is_some_and(|error| !error.is_null())
                    || response
                        .get("status")
                        .and_then(Value::as_u64)
                        .is_some_and(|status| status >= 300)
                    || partial_search_response(response)
            })
            .count();
        if failed > 0 {
            return Some(format!(
                "Elasticsearch msearch: {failed} of {} search(es) failed or returned incomplete results; inspect response.responses.",
                responses.len()
            ));
        }
    }
    if let Some(docs) = value.get("docs").and_then(Value::as_array) {
        let failed = docs
            .iter()
            .filter(|doc| doc.get("error").is_some_and(|error| !error.is_null()))
            .count();
        if failed > 0 {
            return Some(format!(
                "Elasticsearch mget: {failed} of {} document(s) failed; inspect response.docs.",
                docs.len()
            ));
        }
    }
    if let Some(failures) = value.get("failures").and_then(Value::as_array)
        && !failures.is_empty()
    {
        return Some(format!(
            "Elasticsearch reported {} failure(s). Successful operations were not rolled back; inspect response.failures before retrying.",
            failures.len()
        ));
    }
    if partial_search_response(value) {
        return Some("Elasticsearch returned incomplete search results (timeout or shard failure); inspect the response metadata.".into());
    }
    None
}

pub(super) fn partial_search_response(value: &Value) -> bool {
    value.get("timed_out").and_then(Value::as_bool) == Some(true)
        || value
            .pointer("/_shards/failed")
            .and_then(Value::as_u64)
            .is_some_and(|failed| failed > 0)
}

pub(crate) fn outcome(value: Value, budget: QueryBudget) -> Result<QueryOutcome, DatabaseError> {
    let response_error = response_error(&value);
    let mut result = QueryOutcomeAccumulator::with_budget(budget);
    if let Some(hits) = value.pointer("/hits/hits").and_then(Value::as_array) {
        let mut names = BTreeSet::new();
        for hit in hits {
            if let Some(source) = hit.get("_source").and_then(Value::as_object) {
                names.extend(source.keys().cloned());
            }
        }
        // Keep source in its own JSON column, including fields named _id/_index.
        let columns: Vec<_> = ["_index", "_id", "_score", "_source", "_hit"]
            .into_iter()
            .map(|name| ColumnMeta {
                name: name.into(),
                type_name: "JSON".into(),
            })
            .chain(names.iter().map(|name| ColumnMeta {
                name: format!("_source.{name}"),
                type_name: "JSON".into(),
            }))
            .collect();
        result.metadata(columns.clone());
        for (position, hit) in hits.iter().enumerate() {
            if result.budget_exhausted() {
                // Preserve fetched-row accounting without constructing discarded cells.
                for _ in position..hits.len() {
                    result.row(Vec::new(), Vec::new());
                }
                break;
            }
            let mut row: Vec<_> = ["_index", "_id", "_score", "_source", "_hit"]
                .into_iter()
                .map(|name| {
                    if name == "_hit" {
                        CellValue::Text(hit.to_string())
                    } else {
                        cell(&hit[name])
                    }
                })
                .collect();
            row.extend(names.iter().map(|name| cell(&hit["_source"][name])));
            result.row(columns.clone(), row);
        }
        result.done(0);
        // Keep aggregations, shard failures, total hits, and other metadata available.
        let mut metadata = value;
        if let Some(hits) = metadata.get_mut("hits").and_then(Value::as_object_mut) {
            hits.remove("hits");
        }
        json_result(&mut result, &metadata);
    } else {
        json_result(&mut result, &value);
    }
    let mut result = result.finish();
    result.stats.response_error = response_error;
    Ok(result)
}

fn json_result(result: &mut QueryOutcomeAccumulator, value: &Value) {
    let columns = vec![ColumnMeta {
        name: "response".into(),
        type_name: "JSON".into(),
    }];
    result.metadata(columns.clone());
    result.row(columns, vec![CellValue::Text(value.to_string())]);
    result.done(0);
}

fn cell(value: &Value) -> CellValue {
    match value {
        Value::Null => CellValue::Null,
        Value::Bool(value) => CellValue::Boolean(*value),
        Value::Number(value) => value
            .as_i64()
            .map(CellValue::Integer)
            .or_else(|| value.as_u64().map(CellValue::Unsigned))
            .or_else(|| value.as_f64().map(CellValue::Float))
            .unwrap_or_else(|| CellValue::Text(value.to_string())),
        Value::String(value) => CellValue::Text(value.clone()),
        _ => CellValue::Text(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_native_requests_and_classifies_writes() {
        assert!(
            ElasticsearchRequest::parse("POST /books/_search\n{\"query\":{\"match_all\":{}}}")
                .unwrap()
                .is_read_only()
        );
        assert!(
            !ElasticsearchRequest::parse("PUT /books/_doc/1\n{\"title\":\"Rust\"}")
                .unwrap()
                .is_read_only()
        );
        assert!(ElasticsearchRequest::parse("GET /_search/../_refresh").is_err());
        assert!(
            !ElasticsearchRequest::parse("POST /books/_delete_by_query\n{}")
                .unwrap()
                .is_read_only()
        );
        for input in [
            "GET https://example.com",
            "GET //example.com",
            "GET /foo extra",
            "GET /\\example.com",
            "POST /a\n{}\n{}",
        ] {
            assert!(ElasticsearchRequest::parse(input).is_err(), "{input}");
        }
        assert!(ElasticsearchRequest::parse("GET /books/_doc/why%3F").is_ok());
        assert!(ElasticsearchRequest::parse("GET /books/_doc/hash%23").is_ok());
        let bulk = ElasticsearchRequest::parse(
            "POST /_bulk\n{\"index\":{\"_index\":\"books\"}}\n{\"x\":1}",
        )
        .unwrap();
        assert!(bulk.body.unwrap().ends_with('\n'));
    }

    #[test]
    fn batch_failures_preserve_json_and_survive_result_budget_truncation() {
        for (value, expected) in [
            (
                json!({"errors":true,"items":[{"index":{"status":201}},{"create":{"status":409,"error":{"type":"version_conflict_engine_exception"}}}]}),
                "bulk: 1 of 2",
            ),
            (
                json!({"responses":[{"hits":{"hits":[]}},{"status":404,"error":{"type":"index_not_found_exception"}}]}),
                "msearch: 1 of 2",
            ),
            (
                json!({"responses":[{"timed_out":true},{"_shards":{"failed":1}}]}),
                "msearch: 2 of 2",
            ),
            (json!({"errors":true}), "bulk reported errors"),
            (
                json!({"docs":[{"_id":"1","found":false},{"_id":"2","error":{"type":"index_not_found_exception"}}]}),
                "mget: 1 of 2",
            ),
            (
                json!({"updated":1,"failures":[{"cause":{"type":"version_conflict_engine_exception"}}]}),
                "1 failure(s)",
            ),
            (
                json!({"timed_out":true,"hits":{"hits":[]}}),
                "incomplete search results",
            ),
        ] {
            let result = outcome(value.clone(), QueryBudget::UNBOUNDED).unwrap();
            assert!(
                result
                    .stats
                    .response_error
                    .as_deref()
                    .unwrap()
                    .contains(expected)
            );
            let agent_result = crate::agent::types::QueryOutcomeJson::from(result.clone());
            assert!(
                serde_json::to_value(agent_result).unwrap()["response_error"]
                    .as_str()
                    .unwrap()
                    .contains(expected)
            );
            if value.get("hits").is_none() {
                assert_eq!(
                    serde_json::from_str::<Value>(
                        &result.result_sets[0].rows[0][0].clipboard_text()
                    )
                    .unwrap(),
                    value
                );
            }
            let limited = outcome(
                value,
                QueryBudget {
                    max_rows: 0,
                    max_bytes: 0,
                },
            )
            .unwrap();
            assert!(limited.stats.truncated);
            assert!(
                limited
                    .stats
                    .response_error
                    .as_deref()
                    .unwrap()
                    .contains(expected)
            );
        }
        for value in [
            json!({"errors":false,"items":[{"index":{"status":201}},{"delete":{"status":200}}]}),
            json!({"docs":[{"_id":"1","found":false},{"_id":"2","found":true}]}),
            json!({"updated":2,"failures":[]}),
            json!({"responses":[{"hits":{"hits":[]},"timed_out":false,"_shards":{"failed":0}}]}),
        ] {
            assert!(
                outcome(value, QueryBudget::UNBOUNDED)
                    .unwrap()
                    .stats
                    .response_error
                    .is_none()
            );
        }
    }

    #[test]
    fn preserves_documents_metadata_and_budget() {
        let value = json!({"hits":{"total":{"value":2,"relation":"eq"},"hits":[{"_index":"books","_id":"1","_source":{"_id":"source-id","nested":{"a":1}}},{"_id":"2","_source":{"other":true}}]},"aggregations":{"count":{"value":2}}});
        let result = outcome(value.clone(), QueryBudget::UNBOUNDED).unwrap();
        assert_eq!(result.result_sets.len(), 2);
        assert_eq!(result.result_sets[0].rows.len(), 2);
        assert!(
            result.result_sets[0]
                .columns
                .iter()
                .any(|column| column.name == "_source._id")
        );
        assert!(
            result.result_sets[1].rows[0][0]
                .clipboard_text()
                .contains("aggregations")
        );
        let limited = outcome(
            value,
            QueryBudget {
                max_rows: 1,
                max_bytes: 1024 * 1024,
            },
        )
        .unwrap();
        assert!(limited.stats.truncated);
        assert_eq!(limited.stats.row_count, 1);
        assert_eq!(limited.stats.fetched_row_count, 3);
    }
}
