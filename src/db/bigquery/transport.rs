use std::{sync::Arc, time::Duration};

use futures_util::StreamExt;
use reqwest::{Client, Method, StatusCode, header};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    db::{DatabaseError, ErrorCategory, ServerInfo},
    profile::{CatalogScope, ConnectionProfile, DatabaseKind},
    security::sanitize_terminal_text,
};

use super::auth::Credentials;

pub(super) const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_FETCH_BYTES: usize = 128 * 1024 * 1024;

#[cfg(test)]
#[path = "live_tests.rs"]
mod live_tests;

#[cfg(test)]
static TEST_ENDPOINT: std::sync::OnceLock<url::Url> = std::sync::OnceLock::new();

#[derive(Clone)]
pub struct BigQueryAdapter {
    pub(super) client: Client,
    endpoint: url::Url,
    credentials: Credentials,
    pub(crate) profile_id: Uuid,
    pub(crate) project: String,
    pub(crate) data_project: String,
    pub(crate) dataset: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) catalog_scope: CatalogScope,
    pub(super) maximum_bytes_billed: u64,
    pub(super) timeout: Duration,
    pub(super) read_only: bool,
}

impl std::fmt::Debug for BigQueryAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BigQueryAdapter")
            .field("profile_id", &self.profile_id)
            .field("project", &self.project)
            .field("data_project", &self.data_project)
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl BigQueryAdapter {
    pub async fn connect(profile: &ConnectionProfile) -> Result<Self, DatabaseError> {
        let adapter = Self::from_profile(profile).await?;
        adapter.probe().await?;
        Ok(adapter)
    }

    async fn from_profile(profile: &ConnectionProfile) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::BigQuery {
            return Err(DatabaseError::configuration(
                "BigQuery adapter requires a BigQuery profile",
            ));
        }
        profile
            .validate_bigquery()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let options = profile.bigquery_options.clone().unwrap_or_default();
        let project = profile.database.clone().ok_or_else(|| {
            DatabaseError::configuration("BigQuery execution project is required")
        })?;
        #[cfg(test)]
        let fixture_endpoint = TEST_ENDPOINT.get().cloned();
        #[cfg(test)]
        let credentials = if fixture_endpoint.is_some() {
            Credentials::for_test()
        } else {
            Credentials::new(options.credentials_file.as_deref()).await?
        };
        #[cfg(not(test))]
        let credentials = Credentials::new(options.credentials_file.as_deref()).await?;
        let endpoint = url::Url::parse("https://bigquery.googleapis.com/bigquery/v2/")
            .expect("static Google API endpoint");
        #[cfg(test)]
        let endpoint = fixture_endpoint.unwrap_or(endpoint);
        Ok(Self {
            client: http_client()?,
            endpoint,
            credentials,
            profile_id: profile.id,
            data_project: options.effective_data_project(&project).to_owned(),
            project,
            dataset: profile.default_schema.clone(),
            location: options.location.clone(),
            catalog_scope: profile.catalog_scope.clone(),
            maximum_bytes_billed: options.maximum_bytes_billed,
            timeout: Duration::from_secs(options.query_timeout_secs),
            read_only: profile.read_only,
        })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        // A constant query verifies jobs.create and the selected billing project without scanning data.
        self.execute("SELECT 1 AS lazydb_connection_test").await?;
        Ok(ServerInfo {
            kind: DatabaseKind::BigQuery,
            version: "Google BigQuery (GoogleSQL)".into(),
            database: self.project.clone(),
            current_user: None,
        })
    }

    fn url(&self, path: &[&str], params: &[(&str, String)]) -> Result<url::Url, DatabaseError> {
        let mut url = self.endpoint.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| invalid_response("invalid BigQuery endpoint"))?;
            segments.pop_if_empty();
            for part in path {
                if part.is_empty() || matches!(*part, "." | "..") || part.contains('\0') {
                    return Err(DatabaseError::configuration(
                        "invalid BigQuery API path component",
                    ));
                }
                segments.push(part);
            }
        }
        url.query_pairs_mut()
            .extend_pairs(params.iter().map(|(key, value)| (*key, value.as_str())));
        Ok(url)
    }

    pub(super) async fn request(
        &self,
        method: Method,
        path: &[&str],
        params: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<(Value, usize), DatabaseError> {
        let url = self.url(path, params)?;
        let auth = self.credentials.authorization().await?;
        let empty_post = method == Method::POST && body.is_none();
        let mut request = self
            .client
            .request(method, url)
            .header(header::AUTHORIZATION, auth)
            .header(header::ACCEPT, "application/json")
            .header(
                header::USER_AGENT,
                concat!("lazydb/", env!("CARGO_PKG_VERSION")),
            );
        if let Some(body) = body {
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
        } else if empty_post {
            // Google rejects bodyless jobs.cancel POSTs without an explicit length.
            request = request.header(header::CONTENT_LENGTH, "0");
        }
        let response = request.send().await.map_err(network_error)?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_RESPONSE_BYTES
        } else {
            16 * 1024
        };
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err(if status.is_success() {
                invalid_response("BigQuery response exceeds the 16 MiB response limit")
            } else {
                api_error(status, &Value::Null)
            });
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(network_error)?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(if status.is_success() {
                    invalid_response("BigQuery response exceeds the 16 MiB response limit")
                } else {
                    api_error(status, &Value::Null)
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            if status.is_success() {
                invalid_response("BigQuery returned invalid JSON")
            } else {
                api_error(status, &Value::Null)
            }
        })?;
        if !status.is_success() {
            return Err(api_error(status, &value));
        }
        Ok((value, bytes.len()))
    }

    pub(super) async fn get(
        &self,
        path: &[&str],
        params: &[(&str, String)],
    ) -> Result<Value, DatabaseError> {
        self.get_with_size(path, params)
            .await
            .map(|(value, _)| value)
    }

    pub(super) async fn get_with_size(
        &self,
        path: &[&str],
        params: &[(&str, String)],
    ) -> Result<(Value, usize), DatabaseError> {
        // Only safe GETs are retried. SQL submission is never replayed.
        let mut delay = Duration::from_millis(100);
        for attempt in 0..4 {
            match self.request(Method::GET, path, params, None).await {
                Ok(response) => return Ok(response),
                Err(error) if attempt < 3 && retryable(&error) => {
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    pub(super) fn job_body(&self, sql: &str, job_id: &str) -> Value {
        let mut body = json!({
            "jobReference": {"projectId": self.project, "jobId": job_id},
            "configuration": {
                "jobTimeoutMs": self.timeout.as_millis().to_string(),
                "query": {
                    "query": sql,
                    "useLegacySql": false,
                    "maximumBytesBilled": self.maximum_bytes_billed.to_string()
                }
            }
        });
        if let Some(location) = &self.location {
            body["jobReference"]["location"] = json!(location);
        }
        if let Some(dataset) = &self.dataset {
            body["configuration"]["query"]["defaultDataset"] =
                json!({"projectId": self.data_project, "datasetId": dataset});
        }
        body
    }

    pub(super) async fn cancel_job(&self, job: &JobIdentity, cause: &str) -> DatabaseError {
        let params = job.params();
        let cancel = tokio::time::timeout(
            Duration::from_secs(10),
            self.request(
                Method::POST,
                &["projects", &job.project, "jobs", &job.id, "cancel"],
                &params,
                None,
            ),
        )
        .await;
        let accepted = matches!(cancel, Ok(Ok(_)));
        let state = tokio::time::timeout(
            Duration::from_secs(5),
            self.get(&["projects", &job.project, "jobs", &job.id], &params),
        )
        .await;
        let final_state = match state {
            Ok(Ok(value))
                if value.pointer("/status/state").and_then(Value::as_str) == Some("DONE") =>
            {
                if value
                    .pointer("/status/errorResult/reason")
                    .and_then(Value::as_str)
                    == Some("stopped")
                {
                    "remote job confirmed stopped"
                } else if value.pointer("/status/errorResult").is_some() {
                    "remote job finished with an error"
                } else {
                    "remote job completed; results were not collected"
                }
            }
            _ => "remote final state unknown; the job may still run and incur charges",
        };
        DatabaseError {
            category: ErrorCategory::Cancelled,
            code: Some(
                if accepted {
                    "cancel_requested"
                } else {
                    "local_wait_stopped"
                }
                .into(),
            ),
            message: format!(
                "BigQuery {cause}; local waiting stopped; {}; {final_state} (project {}, job {}, location {})",
                if accepted {
                    "cancellation request accepted"
                } else {
                    "cancellation request was not confirmed"
                },
                sanitize_terminal_text(&job.project),
                sanitize_terminal_text(&job.id),
                sanitize_terminal_text(job.location.as_deref().unwrap_or("unspecified"))
            ),
            diagnostic: None,
        }
    }

    pub async fn close(&self) {}

    #[cfg(test)]
    pub(crate) fn install_test_fixture_endpoint(endpoint: url::Url) -> Result<(), DatabaseError> {
        if endpoint.scheme() != "http"
            || endpoint.host_str() != Some("127.0.0.1")
            || endpoint.port().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/bigquery/v2/"
        {
            return Err(DatabaseError::configuration(
                "BigQuery PTY fixture requires an explicit loopback HTTP endpoint",
            ));
        }
        TEST_ENDPOINT.set(endpoint).map_err(|_| {
            DatabaseError::configuration("BigQuery PTY fixture endpoint already installed")
        })
    }

    #[cfg(test)]
    pub(super) fn fixture(endpoint: url::Url) -> Self {
        Self {
            client: http_client().expect("fixture HTTP client"),
            endpoint,
            credentials: Credentials::for_test(),
            profile_id: Uuid::nil(),
            project: "billing-project".into(),
            data_project: "data-project".into(),
            dataset: Some("fixture".into()),
            location: Some("US".into()),
            catalog_scope: CatalogScope {
                databases: crate::profile::CatalogSelection::All,
            },
            maximum_bytes_billed: 1_000_000_000,
            timeout: Duration::from_secs(120),
            read_only: false,
        }
    }
}

pub(super) fn http_client() -> Result<Client, DatabaseError> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .gzip(true)
        .build()
        .map_err(|_| DatabaseError::configuration("failed to initialize BigQuery HTTP client"))
}

#[derive(Clone, Debug)]
pub(super) struct JobIdentity {
    pub project: String,
    pub id: String,
    pub location: Option<String>,
}

impl JobIdentity {
    pub fn params(&self) -> Vec<(&'static str, String)> {
        self.location
            .iter()
            .map(|location| ("location", location.clone()))
            .collect()
    }

    pub fn accept_reference(&mut self, value: &Value) -> Result<(), DatabaseError> {
        if let Some(reference) = value.get("jobReference") {
            if reference.get("projectId").and_then(Value::as_str) != Some(self.project.as_str())
                || reference.get("jobId").and_then(Value::as_str) != Some(self.id.as_str())
            {
                return Err(invalid_response(
                    "BigQuery returned a different job identity",
                ));
            }
            if let Some(location) = reference.get("location").and_then(Value::as_str) {
                if self
                    .location
                    .as_ref()
                    .is_some_and(|expected| !expected.eq_ignore_ascii_case(location))
                {
                    return Err(invalid_response(
                        "BigQuery returned a different job location",
                    ));
                }
                self.location = Some(location.to_owned());
            }
        }
        Ok(())
    }
}

// Runtime cooperatively cancels, but callers may also drop an in-flight future.
// In that case best-effort native cancellation still uses the original job identity.
pub(super) struct JobGuard {
    pub adapter: Arc<BigQueryAdapter>,
    pub job: JobIdentity,
    pub armed: bool,
}
impl Drop for JobGuard {
    fn drop(&mut self) {
        if self.armed
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let adapter = self.adapter.clone();
            let job = self.job.clone();
            runtime.spawn(async move {
                let _ = adapter.cancel_job(&job, "request was dropped").await;
            });
        }
    }
}

pub(super) async fn cancelled(cancel: &CancellationToken) {
    cancel.cancelled().await;
}

pub(crate) fn invalid_response(message: impl AsRef<str>) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Sql,
        code: Some("bigquery_response".into()),
        message: sanitize_terminal_text(message.as_ref()),
        diagnostic: None,
    }
}

fn network_error(error: reqwest::Error) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Network,
        code: Some(
            if error.is_timeout() {
                "timeout"
            } else {
                "network"
            }
            .into(),
        ),
        message: if error.is_timeout() {
            "BigQuery HTTP request timed out"
        } else {
            "BigQuery HTTP request failed"
        }
        .into(),
        diagnostic: None,
    }
}

pub(super) fn retryable(error: &DatabaseError) -> bool {
    error.category == ErrorCategory::Network
        || error.code.as_deref().is_some_and(|code| {
            matches!(
                code,
                "429"
                    | "500"
                    | "502"
                    | "503"
                    | "504"
                    | "backendError"
                    | "internalError"
                    | "rateLimitExceeded"
            )
        })
}

pub(super) fn api_error(status: StatusCode, value: &Value) -> DatabaseError {
    let reason = value
        .pointer("/error/errors/0/reason")
        .and_then(Value::as_str);
    // API messages may echo user-supplied tokens or credential material. Keep remote messages
    // only for SQL/permission errors, and scrub common credential markers before display.
    let category = match status.as_u16() {
        401 => ErrorCategory::Authentication,
        403 => ErrorCategory::Permission,
        408 | 429 | 500..=599 => ErrorCategory::Network,
        _ => ErrorCategory::Sql,
    };
    let message = if category == ErrorCategory::Authentication {
        "BigQuery authentication rejected; renew ADC credentials and reconnect".to_owned()
    } else {
        let detail = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Google API request failed");
        format!(
            "BigQuery HTTP {}: {}",
            status.as_u16(),
            safe_message(detail)
        )
    };
    DatabaseError {
        category,
        code: Some(
            reason
                .map(safe_reason)
                .unwrap_or_else(|| status.as_u16().to_string()),
        ),
        message,
        diagnostic: None,
    }
}

pub(super) fn safe_reason(reason: &str) -> String {
    if !reason.is_empty()
        && reason.len() <= 64
        && reason.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        reason.to_owned()
    } else {
        "bigquery_error".into()
    }
}

pub(super) fn safe_message(message: &str) -> String {
    let lowered = message.to_ascii_lowercase();
    if [
        "access_token",
        "refresh_token",
        "private_key",
        "private key",
        "bearer ",
        "ya29.",
        "-----begin",
        "client_secret",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        "server error details redacted because they contain credential material".into()
    } else {
        sanitize_terminal_text(&message.chars().take(4096).collect::<String>())
    }
}
