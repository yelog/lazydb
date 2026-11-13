use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, Proxy, StatusCode, header};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    db::{
        DatabaseError, ErrorCategory, ServerInfo,
        transport::{ConnectionRoute, TransportEndpoint},
    },
    profile::{ConnectionProfile, DatabaseKind, SslMode},
    security::sanitize_terminal_text,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct ClickHouseAdapter {
    client: Client,
    endpoint: String,
    database: String,
    pub(crate) catalog_scope: crate::profile::CatalogScope,
    user: String,
    password: Option<SecretString>,
    profile_id: uuid::Uuid,
    route: ConnectionRoute,
}

impl ClickHouseAdapter {
    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        let route = ConnectionRoute::require_direct(profile)?;
        Self::connect_routed(profile, password, route).await
    }

    pub(crate) async fn connect_routed(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
        route: ConnectionRoute,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::ClickHouse {
            return Err(DatabaseError::configuration(
                "ClickHouse adapter requires a ClickHouse profile",
            ));
        }
        if !matches!(
            &route.endpoint,
            TransportEndpoint::Direct | TransportEndpoint::Socks5(_)
        ) {
            return Err(DatabaseError::unsupported(
                "ClickHouse connections support only direct or SOCKS5 transport",
            ));
        }
        let host = profile
            .host
            .as_deref()
            .filter(|host| !host.trim().is_empty())
            .ok_or_else(|| DatabaseError::configuration("ClickHouse host is required"))?;
        let port = profile.port.unwrap_or(match profile.ssl_mode {
            SslMode::Disable => 8123,
            _ => 8443,
        });
        if port == 0 {
            return Err(DatabaseError::configuration(
                "ClickHouse port must be non-zero",
            ));
        }
        let scheme = match profile.ssl_mode {
            SslMode::Disable => "http",
            SslMode::Prefer | SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => "https",
        };
        let authority_host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let endpoint = format!("{scheme}://{authority_host}:{port}/");
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT);
        if let TransportEndpoint::Socks5(proxy) = &route.endpoint {
            builder = builder
                .no_proxy()
                .proxy(Proxy::all(format!("socks5h://{proxy}")).map_err(|_| {
                    DatabaseError::configuration("invalid ClickHouse SOCKS5 proxy endpoint")
                })?);
        }
        let client = builder
            .build()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let adapter = Self {
            client,
            endpoint,
            database: profile
                .database
                .clone()
                .filter(|database| !database.is_empty())
                .unwrap_or_else(|| "default".to_owned()),
            catalog_scope: profile.catalog_scope.clone(),
            user: profile
                .user
                .clone()
                .filter(|user| !user.is_empty())
                .unwrap_or_else(|| "default".to_owned()),
            password: password.cloned(),
            profile_id: profile.id,
            route,
        };
        adapter.probe().await?;
        Ok(adapter)
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let (status, body) = self
            .identity_response(
                "SELECT version(), currentDatabase(), currentUser() FORMAT TabSeparated",
            )
            .await?;
        if !status.is_success() {
            return Err(http_error(status, &body));
        }
        let text = std::str::from_utf8(&body)
            .map_err(|_| DatabaseError::configuration("ClickHouse probe returned invalid UTF-8"))?;
        let mut fields = text.trim_end_matches(['\r', '\n']).split('\t');
        let version = fields.next().unwrap_or_default();
        let database = fields.next().unwrap_or_default();
        let current_user = fields.next().unwrap_or_default();
        if version.is_empty() || database.is_empty() || current_user.is_empty() {
            return Err(DatabaseError::configuration(
                "ClickHouse probe returned an incomplete identity row",
            ));
        }
        Ok(ServerInfo {
            kind: DatabaseKind::ClickHouse,
            version: sanitize_terminal_text(version),
            database: sanitize_terminal_text(database),
            current_user: Some(sanitize_terminal_text(current_user)),
        })
    }

    async fn request(&self, sql: &str) -> Result<reqwest::Response, DatabaseError> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .query(&[("database", self.database.as_str())])
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header("X-ClickHouse-User", &self.user)
            .body(sql.to_owned());
        if let Some(password) = &self.password {
            let mut value =
                header::HeaderValue::from_str(password.expose_secret()).map_err(|_| {
                    DatabaseError::configuration(
                        "ClickHouse password contains invalid HTTP header bytes",
                    )
                })?;
            value.set_sensitive(true);
            request = request.header("X-ClickHouse-Key", value);
        }
        let route = self.route.clone();
        route
            .guard(async move {
                request
                    .send()
                    .await
                    .map_err(|error| network_error(error.to_string()))
            })
            .await
    }

    pub(crate) async fn query_response(
        &self,
        sql: &str,
    ) -> Result<reqwest::Response, DatabaseError> {
        let response = self.request(sql).await?;
        if response.status().is_success() {
            Ok(response)
        } else {
            let status = response.status();
            Err(http_error(
                status,
                &read_bounded_error_body(response).await?,
            ))
        }
    }

    async fn identity_response(&self, sql: &str) -> Result<(StatusCode, Vec<u8>), DatabaseError> {
        let response = self.request(sql).await?;
        let status = response.status();
        if !status.is_success() {
            let body = read_bounded_error_body(response).await?;
            return Ok((status, body));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| network_error(error.to_string()))?;
            body.extend_from_slice(&chunk);
        }
        Ok((status, body))
    }

    pub async fn close(self) {
        drop(self);
    }

    pub(crate) fn profile_id(&self) -> uuid::Uuid {
        self.profile_id
    }

    pub(crate) fn server_database(&self) -> &str {
        &self.database
    }
}

async fn read_bounded_error_body(response: reqwest::Response) -> Result<Vec<u8>, DatabaseError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| network_error(error.to_string()))?;
        let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if body.len() == MAX_ERROR_BODY_BYTES {
            break;
        }
    }
    Ok(body)
}

fn http_error(status: StatusCode, body: &[u8]) -> DatabaseError {
    let excerpt = String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_BODY_BYTES)]);
    let message = sanitize_terminal_text(excerpt.trim());
    let upper = excerpt.to_ascii_uppercase();
    let category = if upper.contains("ACCESS_DENIED")
        || upper.contains("NOT ENOUGH PRIVILEGES")
        || upper.contains("CODE: 497")
    {
        ErrorCategory::Permission
    } else {
        match status {
            StatusCode::UNAUTHORIZED => ErrorCategory::Authentication,
            StatusCode::FORBIDDEN => ErrorCategory::Permission,
            StatusCode::TOO_MANY_REQUESTS => ErrorCategory::Network,
            _ => ErrorCategory::Sql,
        }
    };
    DatabaseError {
        category,
        code: None,
        message: if message.is_empty() {
            format!("ClickHouse HTTP request failed with status {status}")
        } else {
            format!("ClickHouse HTTP {status}: {message}")
        },
        diagnostic: None,
    }
}

fn network_error(message: impl AsRef<str>) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Network,
        code: None,
        message: sanitize_terminal_text(message.as_ref()),
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{SshOptions, import_connection_url};

    #[tokio::test]
    async fn direct_connect_rejects_ssh_profiles_before_network_io() {
        let mut imported =
            import_connection_url("clickhouse://default@127.0.0.1:1/default", Some("secret"))
                .unwrap();
        imported.profile.ssh = Some(SshOptions::default());

        let error = ClickHouseAdapter::connect(&imported.profile, None)
            .await
            .unwrap_err();

        assert_eq!(error.category, ErrorCategory::Configuration);
    }

    #[tokio::test]
    async fn tcp_route_fails_closed_before_network_io() {
        let imported =
            import_connection_url("clickhouse://default@127.0.0.1:1/default", Some("secret"))
                .unwrap();
        let route = ConnectionRoute {
            endpoint: TransportEndpoint::Tcp("127.0.0.1:18123".parse().unwrap()),
            lease: None,
        };

        let error = ClickHouseAdapter::connect_routed(&imported.profile, None, route)
            .await
            .unwrap_err();

        assert_eq!(error.category, ErrorCategory::Unsupported);
    }
}
