//! BigQuery authentication using the official Google Cloud credential providers.
//!
//! `google-cloud-auth =1.4.0` is the last official release using reqwest 0.12
//! (Rust 1.85); 1.7.0 uses reqwest 0.13.2. Unlike the mature `gcp_auth` 0.12
//! default provider chain, the official ADC chain does not fall back to running
//! `gcloud auth print-access-token`. OAuth, signing, caching, and refresh remain
//! the SDK's responsibility. ADC may use metadata or workload identity; those
//! deployments require integration testing in their target environment.
//!
//! Explicit files accept only authorized-user and service-account credentials.
//! Their size and token URI are checked before constructing an SDK provider.
//! ADC discovery is delegated to the official builder and trusts the deployment's
//! ADC configuration; it does not pass through the explicit-file validation.
//!
//! API reference: <https://docs.rs/google-cloud-auth/1.4.0/google_cloud_auth/credentials/index.html>

use std::fmt;

use google_cloud_auth::credentials::{
    AccessTokenCredentials, Builder, service_account, user_account,
};
use reqwest::header;
use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::db::{DatabaseError, ErrorCategory};

const BIGQUERY_SCOPE: &str = "https://www.googleapis.com/auth/bigquery";
const GOOGLE_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const LEGACY_GOOGLE_TOKEN_URI: &str = "https://accounts.google.com/o/oauth2/token";
const MAX_CREDENTIAL_BYTES: usize = 1024 * 1024;
const INVALID_CREDENTIALS: &str = "Invalid BigQuery credentials file";
const TOKEN_FAILURE: &str = "Unable to obtain a BigQuery access token";

/// Clones share the SDK provider and its token cache.
#[derive(Clone)]
pub(super) struct Credentials {
    inner: AccessTokenCredentials,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Credentials { .. }")
    }
}

impl Credentials {
    pub(super) async fn new(credentials_file: Option<&str>) -> Result<Self, DatabaseError> {
        let inner = match credentials_file {
            Some(path) => {
                let json = read_credentials(path).await?;
                match json.get("type").and_then(Value::as_str) {
                    Some("authorized_user") => user_account::Builder::new(json)
                        .with_scopes([BIGQUERY_SCOPE])
                        // Always use Google's default endpoint for user credentials.
                        .with_token_uri(GOOGLE_TOKEN_URI)
                        .build_access_token_credentials()
                        .map_err(|_| DatabaseError::configuration(INVALID_CREDENTIALS))?,
                    Some("service_account") => service_account::Builder::new(json)
                        .with_access_specifier(service_account::AccessSpecifier::from_scopes([
                            BIGQUERY_SCOPE,
                        ]))
                        .build_access_token_credentials()
                        .map_err(|_| DatabaseError::configuration(INVALID_CREDENTIALS))?,
                    _ => return Err(DatabaseError::configuration(INVALID_CREDENTIALS)),
                }
            }
            None => {
                // The official builder reads ADC files synchronously. Keep discovery
                // off the async executor; spawned SDK refresh tasks use this runtime.
                tokio::task::spawn_blocking(|| {
                    Builder::default()
                        .with_scopes([BIGQUERY_SCOPE])
                        .build_access_token_credentials()
                        .map_err(|_| {
                            authentication_error("Unable to discover BigQuery credentials")
                        })
                })
                .await
                .map_err(|_| authentication_error("Unable to discover BigQuery credentials"))??
            }
        };
        Ok(Self { inner })
    }

    pub(super) async fn authorization(&self) -> Result<header::HeaderValue, DatabaseError> {
        self.token().await
    }

    pub(super) async fn token(&self) -> Result<header::HeaderValue, DatabaseError> {
        let token = self
            .inner
            .access_token()
            .await
            .map_err(|_| authentication_error(TOKEN_FAILURE))?;
        if token.token.is_empty() {
            return Err(authentication_error(TOKEN_FAILURE));
        }
        let mut value = header::HeaderValue::from_str(&format!("Bearer {}", token.token))
            .map_err(|_| authentication_error(TOKEN_FAILURE))?;
        value.set_sensitive(true);
        Ok(value)
    }

    #[cfg(test)]
    pub(super) fn for_test() -> Self {
        #[derive(Debug)]
        struct FixtureProvider;

        impl google_cloud_auth::credentials::CredentialsProvider for FixtureProvider {
            async fn headers(
                &self,
                _: http::Extensions,
            ) -> Result<
                google_cloud_auth::credentials::CacheableResource<header::HeaderMap>,
                google_cloud_auth::errors::CredentialsError,
            > {
                Ok(google_cloud_auth::credentials::CacheableResource::NotModified)
            }

            async fn universe_domain(&self) -> Option<String> {
                Some("googleapis.com".to_owned())
            }
        }

        impl google_cloud_auth::credentials::AccessTokenCredentialsProvider for FixtureProvider {
            async fn access_token(
                &self,
            ) -> Result<
                google_cloud_auth::credentials::AccessToken,
                google_cloud_auth::errors::CredentialsError,
            > {
                Ok(google_cloud_auth::credentials::AccessToken {
                    token: "fixture-access-token".to_owned(),
                })
            }
        }

        Self {
            inner: AccessTokenCredentials::from(FixtureProvider),
        }
    }

    /// The only custom token endpoint is compiled exclusively for local tests.
    #[cfg(test)]
    fn user_account_for_test(bytes: &[u8], token_uri: &str) -> Result<Self, DatabaseError> {
        let json = parse_credentials(bytes)?;
        if json.get("type").and_then(Value::as_str) != Some("authorized_user") {
            return Err(DatabaseError::configuration(INVALID_CREDENTIALS));
        }
        let inner = user_account::Builder::new(json)
            .with_scopes([BIGQUERY_SCOPE])
            .with_token_uri(token_uri)
            .build_access_token_credentials()
            .map_err(|_| DatabaseError::configuration(INVALID_CREDENTIALS))?;
        Ok(Self { inner })
    }
}

async fn read_credentials(path: &str) -> Result<Value, DatabaseError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| DatabaseError::configuration("Unable to read BigQuery credentials file"))?;
    // A bounded read also handles files that grow after opening or have no useful
    // metadata size. Never allocate the entire contents of an untrusted file.
    let mut bytes = Vec::new();
    file.take((MAX_CREDENTIAL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| DatabaseError::configuration("Unable to read BigQuery credentials file"))?;
    parse_credentials(&bytes)
}

fn parse_credentials(bytes: &[u8]) -> Result<Value, DatabaseError> {
    if bytes.len() > MAX_CREDENTIAL_BYTES {
        return Err(DatabaseError::configuration(
            "BigQuery credentials file exceeds 1 MiB",
        ));
    }
    let json: Value = serde_json::from_slice(bytes)
        .map_err(|_| DatabaseError::configuration(INVALID_CREDENTIALS))?;
    if !matches!(
        json.get("type").and_then(Value::as_str),
        Some("authorized_user" | "service_account")
    ) {
        return Err(DatabaseError::configuration(INVALID_CREDENTIALS));
    }
    // Exact endpoints protect refresh tokens and service-account audiences from
    // attacker-controlled hosts, userinfo, ports, redirects encoded in the URI,
    // and alternate schemes. Never echo the rejected URI or JSON in errors.
    if let Some(uri) = json.get("token_uri") {
        match uri.as_str() {
            Some(GOOGLE_TOKEN_URI | LEGACY_GOOGLE_TOKEN_URI) => {}
            _ => {
                return Err(DatabaseError::configuration(
                    "BigQuery credentials must use a Google HTTPS token endpoint",
                ));
            }
        }
    }
    Ok(json)
}

fn authentication_error(message: &'static str) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Authentication,
        code: None,
        message: message.to_owned(),
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    fn authorized_user() -> Value {
        serde_json::json!({
            "type": "authorized_user",
            "client_id": "fixture-client",
            "client_secret": "fixture-secret",
            "refresh_token": "fixture-refresh"
        })
    }

    struct TokenServer {
        uri: String,
        requests: tokio::sync::mpsc::UnboundedReceiver<Value>,
        count: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for TokenServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl TokenServer {
        async fn new(responses: Vec<(u16, Value)>) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let uri = format!("http://{}/token", listener.local_addr().unwrap());
            let (tx, requests) = tokio::sync::mpsc::unbounded_channel();
            let count = Arc::new(AtomicUsize::new(0));
            let counter = count.clone();
            let task = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    let body_offset = loop {
                        let mut chunk = [0; 1024];
                        let size = stream.read(&mut chunk).await.unwrap();
                        assert!(size > 0 && bytes.len() + size <= 16 * 1024);
                        bytes.extend_from_slice(&chunk[..size]);
                        if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            break offset + 4;
                        }
                    };
                    let headers = std::str::from_utf8(&bytes[..body_offset]).unwrap();
                    assert!(headers.starts_with("POST /token HTTP/1.1\r\n"));
                    let length = headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .unwrap()
                        .1
                        .trim()
                        .parse::<usize>()
                        .unwrap();
                    assert!(length <= 8 * 1024);
                    while bytes.len() < body_offset + length {
                        let mut chunk = [0; 1024];
                        let size = stream.read(&mut chunk).await.unwrap();
                        assert!(size > 0);
                        bytes.extend_from_slice(&chunk[..size]);
                    }
                    let request =
                        serde_json::from_slice(&bytes[body_offset..body_offset + length]).unwrap();
                    let index = counter.fetch_add(1, Ordering::SeqCst);
                    let (status, body) = &responses[index.min(responses.len() - 1)];
                    let body = body.to_string();
                    let response = format!(
                        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                    stream.shutdown().await.unwrap();
                    let _ = tx.send(request);
                }
            });
            Self {
                uri,
                requests,
                count,
                task,
            }
        }

        async fn request(&mut self) -> Value {
            tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
                .await
                .expect("local token fixture timed out")
                .expect("local token fixture stopped")
        }
    }

    fn token_response(token: &str) -> (u16, Value) {
        (
            200,
            serde_json::json!({
                "access_token": token,
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
    }

    #[test]
    fn parses_authorized_user_and_official_token_endpoints() {
        let mut json = authorized_user();
        assert_eq!(
            parse_credentials(&serde_json::to_vec(&json).unwrap()).unwrap(),
            json
        );
        for uri in [GOOGLE_TOKEN_URI, LEGACY_GOOGLE_TOKEN_URI] {
            json["token_uri"] = Value::from(uri);
            assert!(parse_credentials(&serde_json::to_vec(&json).unwrap()).is_ok());
        }
    }

    #[test]
    fn rejects_untrusted_token_uris_and_unsupported_json() {
        for kind in ["authorized_user", "service_account"] {
            for uri in [
                Value::from("http://oauth2.googleapis.com/token"),
                Value::from("https://attacker.example/token"),
                Value::from("https://oauth2.googleapis.com.attacker.example/token"),
                Value::from("https://oauth2.googleapis.com@attacker.example/token"),
                Value::from("https://oauth2.googleapis.com/token?redirect=secret"),
                Value::from("https://oauth2.googleapis.com:8443/token"),
                Value::Null,
                Value::from(42),
            ] {
                let mut json = authorized_user();
                json["type"] = Value::from(kind);
                json["token_uri"] = uri;
                let error = parse_credentials(&serde_json::to_vec(&json).unwrap()).unwrap_err();
                assert_eq!(error.category, ErrorCategory::Configuration);
                assert_eq!(
                    error.message,
                    "BigQuery credentials must use a Google HTTPS token endpoint"
                );
            }
        }
        for bytes in [
            b"secret invalid json".as_slice(),
            b"null",
            b"{}",
            b"{\"type\":\"external_account\"}",
        ] {
            assert_eq!(
                parse_credentials(bytes).unwrap_err().message,
                INVALID_CREDENTIALS
            );
        }
    }

    #[tokio::test]
    async fn authorized_user_builder_errors_are_redacted() {
        let mut json = authorized_user();
        json["client_secret"] = Value::from(12345);
        let error = Credentials::user_account_for_test(
            &serde_json::to_vec(&json).unwrap(),
            "http://127.0.0.1:1/token",
        )
        .unwrap_err();
        assert_eq!(error.message, INVALID_CREDENTIALS);
        assert!(!format!("{error:?}").contains("12345"));
    }

    #[tokio::test]
    async fn explicit_file_failures_are_bounded_and_redacted() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("secret-missing-path.json");
        let error = Credentials::new(Some(missing.to_str().unwrap()))
            .await
            .unwrap_err();
        assert_eq!(error.message, "Unable to read BigQuery credentials file");
        assert!(!format!("{error:?}").contains("secret-missing-path"));
        let large = directory.path().join("large.json");
        tokio::fs::write(&large, vec![b' '; MAX_CREDENTIAL_BYTES + 1])
            .await
            .unwrap();
        assert_eq!(
            Credentials::new(Some(large.to_str().unwrap()))
                .await
                .unwrap_err()
                .message,
            "BigQuery credentials file exceeds 1 MiB"
        );

        let key = directory.path().join("service.json");
        tokio::fs::write(
            &key,
            serde_json::to_vec(&serde_json::json!({
                "type": "service_account",
                "client_email": "fixture@example.invalid",
                "private_key_id": "fixture-id",
                "private_key": "secret-malformed-private-key",
                "project_id": "fixture-project",
                "token_uri": GOOGLE_TOKEN_URI
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        // The official service-account builder parses keys lazily when issuing
        // the first self-signed token, so exercise that error as well as parsing.
        let credentials = Credentials::new(Some(key.to_str().unwrap())).await.unwrap();
        let error = credentials.token().await.unwrap_err();
        assert_eq!(error.message, TOKEN_FAILURE);
        assert_eq!(error.category, ErrorCategory::Authentication);
        assert!(!format!("{error:?} {credentials:?}").contains("secret-malformed"));
    }

    #[tokio::test]
    async fn official_refresh_and_cache_are_shared_by_concurrent_clones() {
        let mut server = TokenServer::new(vec![
            token_response("fixture-first"),
            token_response("fixture-second"),
        ])
        .await;
        let credentials = Credentials::user_account_for_test(
            &serde_json::to_vec(&authorized_user()).unwrap(),
            &server.uri,
        )
        .unwrap();
        let first = credentials.authorization().await.unwrap();
        assert_eq!(first, "Bearer fixture-first");
        assert!(first.is_sensitive());
        assert!(!format!("{credentials:?} {first:?}").contains("fixture-first"));
        let request = server.request().await;
        assert_eq!(request["grant_type"], "refresh_token");
        assert_eq!(request["refresh_token"], "fixture-refresh");
        assert_eq!(request["client_id"], "fixture-client");
        assert_eq!(request["client_secret"], "fixture-secret");
        assert_eq!(request["scopes"], BIGQUERY_SCOPE);

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let credentials = credentials.clone();
            tasks.spawn(async move { credentials.token().await.unwrap() });
        }
        while let Some(result) = tasks.join_next().await {
            assert_eq!(result.unwrap(), first);
        }
        assert_eq!(server.count.load(Ordering::SeqCst), 1);

        // The SDK refreshes in the background. Advance beyond expiry without
        // sleeping, then resume real time before doing loopback HTTP I/O.
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(3601)).await;
        tokio::time::resume();
        let refreshed = tokio::time::timeout(Duration::from_secs(5), credentials.token())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(refreshed, "Bearer fixture-second");
        assert!(refreshed.is_sensitive());
        server.request().await;
        assert_eq!(server.count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn token_response_errors_and_invalid_headers_are_redacted() {
        for response in [
            (
                400,
                serde_json::json!({"error":"invalid_grant", "error_description":"secret-server-detail"}),
            ),
            token_response("secret-token\r\ninjected: header"),
            token_response(""),
        ] {
            let mut server = TokenServer::new(vec![response]).await;
            let credentials = Credentials::user_account_for_test(
                &serde_json::to_vec(&authorized_user()).unwrap(),
                &server.uri,
            )
            .unwrap();
            let error = tokio::time::timeout(Duration::from_secs(5), credentials.token())
                .await
                .unwrap()
                .unwrap_err();
            assert_eq!(error.message, TOKEN_FAILURE);
            assert_eq!(error.category, ErrorCategory::Authentication);
            assert!(!format!("{error:?}").contains("secret"));
            server.request().await;
        }
    }
}
