use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, Method, StatusCode, header};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio::time::Instant;
use uuid::Uuid;

use crate::{
    db::{DatabaseError, ErrorCategory, ServerInfo, value::CellValue},
    profile::{CatalogScope, ConnectionProfile, DatabaseKind, SslMode},
    security::sanitize_terminal_text,
};

#[cfg(test)]
mod tests;

const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub struct SnowflakeAdapter {
    client: Client,
    endpoint: url::Url,
    token: SecretString,
    token_type: String,
    pub(crate) profile_id: Uuid,
    pub(crate) database: String,
    schema: Option<String>,
    warehouse: Option<String>,
    role: Option<String>,
    pub(crate) catalog_scope: CatalogScope,
}

impl SnowflakeAdapter {
    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        let adapter = Self::from_profile(profile, password)?;
        adapter.probe().await?;
        Ok(adapter)
    }

    fn from_profile(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::Snowflake {
            return Err(DatabaseError::configuration(
                "Snowflake adapter requires a Snowflake profile",
            ));
        }
        if profile.ssl_mode != SslMode::VerifyFull {
            return Err(DatabaseError::configuration(
                "Snowflake requires HTTPS with certificate verification",
            ));
        }
        let host = profile
            .host
            .as_deref()
            .filter(|host| !host.trim().is_empty())
            .ok_or_else(|| {
                DatabaseError::configuration("Snowflake account hostname is required")
            })?;
        if !host.contains('.')
            || host.contains(['/', '@', '?', '#', ':'])
            || host.chars().any(char::is_whitespace)
        {
            return Err(DatabaseError::configuration(
                "Snowflake requires a full account hostname",
            ));
        }
        let mut endpoint = url::Url::parse("https://localhost/api/v2/statements")
            .expect("static Snowflake API URL");
        endpoint
            .set_host(Some(host))
            .map_err(|_| DatabaseError::configuration("invalid Snowflake hostname"))?;
        let port = profile.port.unwrap_or(443);
        if port != 443 {
            return Err(DatabaseError::configuration(
                "Snowflake requires HTTPS port 443",
            ));
        }
        endpoint
            .set_port(Some(port))
            .map_err(|_| DatabaseError::configuration("invalid Snowflake port"))?;
        let database = profile
            .database
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| DatabaseError::configuration("Snowflake database is required"))?;
        let token = password.filter(|token| !token.expose_secret().is_empty()).cloned()
            .ok_or_else(|| DatabaseError::configuration("Snowflake access token is required (PAT, OAuth, or key-pair JWT; password authentication is not supported)"))?;
        let options = profile.snowflake_options.as_ref();
        if let Some(options) = options {
            options
                .validate()
                .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        }
        let token_type = options
            .map(|options| options.token_type.as_str())
            .unwrap_or("PROGRAMMATIC_ACCESS_TOKEN");
        if !matches!(
            token_type,
            "PROGRAMMATIC_ACCESS_TOKEN" | "OAUTH" | "KEYPAIR_JWT"
        ) {
            return Err(DatabaseError::configuration("invalid Snowflake token type"));
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .gzip(true)
            .build()
            .map_err(|_| {
                DatabaseError::configuration("failed to initialize Snowflake HTTP client")
            })?;
        Ok(Self {
            client,
            endpoint,
            token,
            token_type: token_type.to_owned(),
            profile_id: profile.id,
            database,
            schema: Some(
                profile
                    .default_schema
                    .clone()
                    .unwrap_or_else(|| "PUBLIC".into()),
            ),
            warehouse: options.and_then(|options| options.warehouse.clone()),
            role: options.and_then(|options| options.role.clone()),
            catalog_scope: profile.catalog_scope.clone(),
        })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let result = self
            .execute("SELECT CURRENT_VERSION(), CURRENT_DATABASE(), CURRENT_USER()")
            .await?;
        let row = result
            .result_sets
            .first()
            .and_then(|set| set.rows.first())
            .ok_or_else(|| invalid_response("Snowflake probe returned no identity row"))?;
        let text = |index| match row.get(index) {
            Some(CellValue::Text(value)) if !value.is_empty() => Ok(sanitize_terminal_text(value)),
            _ => Err(invalid_response(
                "Snowflake probe returned an incomplete identity row",
            )),
        };
        Ok(ServerInfo {
            kind: DatabaseKind::Snowflake,
            version: text(0)?,
            database: text(1)?,
            current_user: Some(text(2)?),
        })
    }

    pub(crate) fn statement_body(&self, sql: &str) -> Value {
        // SQL API parameters have an explicit allowlist; AUTOCOMMIT is not accepted.
        // https://docs.snowflake.com/en/developer-guide/sql-api/reference
        let mut body = json!({
            "statement": sql,
            "timeout": QUERY_TIMEOUT.as_secs(),
            "database": self.database,
            "parameters": {
                "MULTI_STATEMENT_COUNT": "1",
                "BINARY_OUTPUT_FORMAT": "HEX",
                "DATE_OUTPUT_FORMAT": "YYYY-MM-DD",
                "TIME_OUTPUT_FORMAT": "HH24:MI:SS.FF9",
                "TIMESTAMP_NTZ_OUTPUT_FORMAT": "YYYY-MM-DD HH24:MI:SS.FF9",
                "TIMESTAMP_LTZ_OUTPUT_FORMAT": "YYYY-MM-DD HH24:MI:SS.FF9 TZH:TZM",
                "TIMESTAMP_TZ_OUTPUT_FORMAT": "YYYY-MM-DD HH24:MI:SS.FF9 TZH:TZM"
            }
        });
        for (name, value) in [
            ("schema", &self.schema),
            ("warehouse", &self.warehouse),
            ("role", &self.role),
        ] {
            if let Some(value) = value {
                body[name] = json!(value);
            }
        }
        body
    }

    async fn request(
        &self,
        method: Method,
        url: url::Url,
        body: Option<&Value>,
    ) -> Result<(StatusCode, Value), DatabaseError> {
        let mut auth =
            header::HeaderValue::from_str(&format!("Bearer {}", self.token.expose_secret()))
                .map_err(|_| {
                    DatabaseError::configuration(
                        "Snowflake token contains invalid HTTP header bytes",
                    )
                })?;
        auth.set_sensitive(true);
        let mut request = self
            .client
            .request(method, url)
            .header(header::AUTHORIZATION, auth)
            .header("X-Snowflake-Authorization-Token-Type", &self.token_type)
            .header(header::ACCEPT, "application/json")
            .header(
                header::USER_AGENT,
                concat!("lazydb/", env!("CARGO_PKG_VERSION")),
            );
        if let Some(body) = body {
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let response = request.send().await.map_err(network_error)?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_RESPONSE_BYTES
        } else {
            16 * 1024
        };
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(network_error)?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                if !status.is_success() {
                    break;
                }
                return Err(invalid_response(
                    "Snowflake response exceeds the 64 MiB partition limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice(&bytes).map_err(|_| {
            if status.is_success() {
                invalid_response("Snowflake returned invalid JSON")
            } else {
                api_error(status, &Value::Null)
            }
        })?;
        Ok((status, value))
    }

    pub(crate) async fn submit(&self, sql: &str) -> Result<Value, DatabaseError> {
        let deadline = Instant::now() + QUERY_TIMEOUT;
        let mut url = self.endpoint.clone();
        url.query_pairs_mut()
            .append_pair("requestId", &Uuid::new_v4().to_string())
            .append_pair("async", "true");
        let (status, value) = self
            .request(Method::POST, url, Some(&self.statement_body(sql)))
            .await?;
        self.await_result(status, value, deadline, None).await
    }

    pub(crate) async fn partition(
        &self,
        handle: &str,
        partition: usize,
    ) -> Result<Value, DatabaseError> {
        let mut url = self.handle_url(handle)?;
        url.query_pairs_mut()
            .append_pair("partition", &partition.to_string());
        let (status, value) = self.request(Method::GET, url, None).await?;
        self.await_result(
            status,
            value,
            Instant::now() + QUERY_TIMEOUT,
            Some((handle, partition)),
        )
        .await
    }

    async fn await_result(
        &self,
        mut status: StatusCode,
        mut value: Value,
        deadline: Instant,
        partition: Option<(&str, usize)>,
    ) -> Result<Value, DatabaseError> {
        // Build polling URLs from validated handles; never forward credentials to response-supplied URLs.
        let handle = partition.map(|(handle, _)| handle.to_owned()).or_else(|| {
            value
                .get("statementHandle")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        let mut backoff = Duration::from_millis(200);
        loop {
            if status == StatusCode::OK {
                return Ok(value);
            }
            if !matches!(status, StatusCode::ACCEPTED | StatusCode::TOO_MANY_REQUESTS) {
                return Err(api_error(status, &value));
            }
            let handle = handle.as_deref().ok_or_else(|| api_error(status, &value))?;
            let mut url = self.handle_url(handle)?;
            if let Some((_, partition)) = partition {
                url.query_pairs_mut()
                    .append_pair("partition", &partition.to_string());
            }
            if Instant::now() + backoff >= deadline {
                // Best effort server cancellation bounds warehouse work on a timed-out query.
                let mut cancel = url.clone();
                cancel
                    .path_segments_mut()
                    .expect("HTTPS endpoint")
                    .push("cancel");
                let _ = self.request(Method::POST, cancel, None).await;
                return Err(DatabaseError {
                    category: ErrorCategory::Network,
                    code: Some("snowflake_query_timeout".into()),
                    message: "Snowflake query exceeded the 300 second polling deadline".into(),
                    diagnostic: None,
                });
            }
            tokio::time::sleep(backoff).await;
            (status, value) = self.request(Method::GET, url, None).await?;
            if let Some(next) = value.get("statementHandle").and_then(Value::as_str) {
                // The original handle remains valid when subsequent status responses omit it.
                if next != handle {
                    return Err(invalid_response(
                        "Snowflake polling response changed the statement handle",
                    ));
                }
            }
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }
    }

    fn handle_url(&self, handle: &str) -> Result<url::Url, DatabaseError> {
        Uuid::parse_str(handle)
            .map_err(|_| invalid_response("Snowflake returned an invalid statement handle"))?;
        let mut url = self.endpoint.clone();
        url.path_segments_mut()
            .expect("HTTPS endpoint")
            .push(handle);
        Ok(url)
    }

    pub async fn close(self) {
        drop(self);
    }
}

pub(crate) fn invalid_response(message: &str) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("snowflake_invalid_response".into()),
        message: message.into(),
        diagnostic: None,
    }
}

fn network_error(error: reqwest::Error) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Network,
        code: None,
        message: if error.is_timeout() {
            "Snowflake HTTP request timed out"
        } else {
            "Snowflake HTTP request failed"
        }
        .into(),
        diagnostic: None,
    }
}

fn api_error(status: StatusCode, body: &Value) -> DatabaseError {
    let state = body
        .get("sqlState")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let code = body.get("code").and_then(Value::as_str);
    let category = match status {
        _ if matches!(code, Some("390111" | "390303" | "390318")) => ErrorCategory::Authentication,
        StatusCode::UNAUTHORIZED => ErrorCategory::Authentication,
        StatusCode::FORBIDDEN => ErrorCategory::Permission,
        StatusCode::TOO_MANY_REQUESTS => ErrorCategory::Network,
        _ if state.starts_with("28") => ErrorCategory::Authentication,
        _ if state == "42501" => ErrorCategory::Permission,
        _ if state.starts_with("23") => ErrorCategory::Constraint,
        _ if status.is_server_error() => ErrorCategory::Network,
        _ => ErrorCategory::Sql,
    };
    DatabaseError {
        category,
        code: body.get("code").and_then(Value::as_str).map(str::to_owned),
        message: sanitize_terminal_text(
            body.get("message")
                .and_then(Value::as_str)
                .unwrap_or("Snowflake SQL API request failed"),
        ),
        diagnostic: None,
    }
}
