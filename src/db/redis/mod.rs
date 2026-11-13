pub(crate) mod connection;
pub mod discovery;
pub mod key_index;
pub mod key_store;
pub mod metadata_cache;
pub mod monitor;
pub mod mutation;
pub mod preview_scheduler;
pub mod read;
pub mod reconnect;
pub mod reply;
pub mod scan_scheduler;
pub mod server_info;
pub mod types;

use redis::{Client, aio::ConnectionManagerConfig};
use secrecy::{ExposeSecret, SecretString};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

use crate::{
    db::{
        DatabaseError, ErrorCategory, ServerInfo,
        transport::{ConnectionRoute, TransportEndpoint},
    },
    profile::{ConnectionProfile, DatabaseKind, SslMode},
};

use self::connection::{
    CONNECTION_TIMEOUT, RESPONSE_TIMEOUT, RedisConnection, is_tunnel_unavailable,
};

#[derive(Clone, Debug)]
pub struct RedisAdapter {
    connection: RedisConnection,
    target_database: u32,
    configured_kind: DatabaseKind,
    connection_id: Uuid,
    metadata_cache: Arc<Mutex<metadata_cache::MetadataCache>>,
}

impl RedisAdapter {
    pub async fn load_monitor_snapshot(
        &self,
    ) -> Result<crate::db::monitor::MonitorSnapshot, DatabaseError> {
        let mut connection = self.connection.clone();
        let raw = redis::cmd("INFO")
            .query_async::<String>(&mut connection)
            .await
            .map_err(|error| redis_error(error, crate::db::ErrorCategory::Network))?;
        let (mut snapshot, details) =
            monitor::parse_info(&raw, chrono::Utc::now().timestamp_millis() as u64);
        snapshot.redis_details = Some(details);
        Ok(snapshot)
    }

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
        if !profile.kind.is_redis_compatible() {
            return Err(DatabaseError::configuration(
                "Redis-compatible adapter requires a Redis or Valkey profile",
            ));
        }
        let host = profile
            .host
            .as_deref()
            .ok_or_else(|| DatabaseError::configuration("Redis-compatible host is required"))?;
        let port = profile.port.unwrap_or(6379);
        let database = parse_database(profile.database.as_deref())?;
        let logical_host = normalized_host(host);
        let scheme = match profile.ssl_mode {
            SslMode::Disable => "redis",
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => "rediss",
            SslMode::Prefer => {
                return Err(DatabaseError::configuration(
                    "Redis TLS mode must be disable or require",
                ));
            }
        };
        let credentials = password
            .map(|password| {
                let user = profile.user.as_deref().unwrap_or("default");
                format!(
                    "{}:{}@",
                    encode_url_part(user),
                    encode_url_part(password.expose_secret())
                )
            })
            .or_else(|| {
                profile
                    .user
                    .as_deref()
                    .map(|user| format!("{}@", encode_url_part(user)))
            })
            .unwrap_or_default();
        let url = format!(
            "{scheme}://{credentials}{}:{port}/{database}",
            url_host(host)
        );
        let client =
            Client::open(url).map_err(|error| redis_error(error, ErrorCategory::Configuration))?;
        let connection = match route.endpoint.clone() {
            TransportEndpoint::Direct => {
                if route.lease.is_some() {
                    return Err(DatabaseError::configuration(
                        "Redis direct transport cannot carry an SSH tunnel lease",
                    ));
                }
                let _ = ConnectionRoute::require_direct(profile)?;
                let reconnect = reconnect::policy();
                let manager_config = ConnectionManagerConfig::new()
                    .set_connection_timeout(Some(CONNECTION_TIMEOUT))
                    .set_response_timeout(Some(RESPONSE_TIMEOUT))
                    .set_number_of_retries(reconnect.attempts)
                    .set_min_delay(reconnect.initial_delay)
                    .set_max_delay(reconnect.max_delay);
                redis::aio::ConnectionManager::new_with_config(client, manager_config)
                    .await
                    .map(RedisConnection::direct)
            }
            TransportEndpoint::Tcp(endpoint) => {
                if route.lease.as_ref().is_some_and(|lease| !lease.is_alive()) {
                    return Err(tunnel_unavailable());
                }
                RedisConnection::connect_routed(
                    client,
                    route,
                    endpoint,
                    logical_host,
                    i64::from(database),
                )
                .await
            }
            TransportEndpoint::Unix(_) => {
                return Err(DatabaseError::unsupported(
                    "Redis routed connections do not support Unix socket transport",
                ));
            }
            TransportEndpoint::Socks5(_) => {
                return Err(DatabaseError::unsupported(
                    "Redis routed connections do not support SOCKS5 transport",
                ));
            }
        }
        .map_err(|error| {
            let category = if matches!(error.kind(), redis::ErrorKind::InvalidClientConfig) {
                ErrorCategory::Configuration
            } else if password.is_some() {
                ErrorCategory::Authentication
            } else {
                ErrorCategory::Network
            };
            redis_error(error, category)
        })?;
        let adapter = Self {
            connection,
            target_database: database,
            configured_kind: profile.kind,
            connection_id: profile.id,
            metadata_cache: Arc::new(Mutex::new(metadata_cache::MetadataCache::new(
                2_048,
                Duration::from_secs(1),
            ))),
        };
        adapter.ping().await?;
        Ok(adapter)
    }

    pub async fn ping(&self) -> Result<(), DatabaseError> {
        let mut connection = self.connection.clone();
        redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .map(|_| ())
            .map_err(|error| redis_error(error, ErrorCategory::Network))
    }

    pub async fn scan_keys(
        &self,
        cursor: u64,
        pattern: &[u8],
        count_hint: u32,
    ) -> Result<(u64, Vec<Vec<u8>>), DatabaseError> {
        let scan = || async {
            let mut connection = self.connection.clone();
            let mut command = redis::cmd("SCAN");
            command.arg(cursor).arg("MATCH").arg(pattern);
            if count_hint > 0 {
                command.arg("COUNT").arg(count_hint);
            }
            command.query_async(&mut connection).await
        };
        match scan().await {
            Ok(result) => Ok(result),
            Err(error) if error.is_connection_dropped() => scan()
                .await
                .map_err(|error| redis_error(error, ErrorCategory::Network)),
            Err(error) => Err(redis_error(error, ErrorCategory::Network)),
        }
    }

    pub async fn delete_key(&self, key: &[u8]) -> Result<u64, DatabaseError> {
        let mut connection = self.connection.clone();
        redis::cmd("DEL")
            .arg(key)
            .query_async(&mut connection)
            .await
            .map_err(|error| redis_error(error, ErrorCategory::Network))
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let mut connection = self.connection.clone();
        let info = redis::cmd("INFO")
            .arg("server")
            .query_async::<String>(&mut connection)
            .await
            .unwrap_or_default();
        let version = server_info::parse(&info)
            .version
            .unwrap_or_else(|| "unknown".to_owned());
        Ok(ServerInfo {
            kind: self.configured_kind,
            version,
            database: self.target_database.to_string(),
            current_user: None,
        })
    }

    pub fn database(&self) -> u32 {
        self.target_database
    }

    pub fn kind(&self) -> DatabaseKind {
        self.configured_kind
    }

    pub async fn close(self) {
        drop(self.connection);
    }

    pub(crate) fn connection_clone(&self) -> RedisConnection {
        self.connection.clone()
    }

    pub(crate) fn invalidate_metadata(&self, key: &[u8]) {
        if let Ok(mut cache) = self.metadata_cache.lock() {
            cache.invalidate_key(
                crate::identity::ConnectionIdentity {
                    profile_id: self.connection_id,
                    generation: 0,
                },
                key,
            );
        }
    }

    pub(crate) fn metadata_cache_key(&self, key: &[u8]) -> metadata_cache::MetadataCacheKey {
        metadata_cache::MetadataCacheKey {
            connection: crate::identity::ConnectionIdentity {
                profile_id: self.connection_id,
                generation: 0,
            },
            key: key.to_vec(),
        }
    }

    pub(crate) fn metadata_cache_get(
        &self,
        key: &metadata_cache::MetadataCacheKey,
    ) -> Option<crate::db::redis::read::RedisKeyMetadata> {
        self.metadata_cache.lock().ok()?.get(key)
    }

    pub(crate) fn metadata_cache_insert(
        &self,
        key: metadata_cache::MetadataCacheKey,
        value: crate::db::redis::read::RedisKeyMetadata,
    ) {
        if let Ok(mut cache) = self.metadata_cache.lock() {
            cache.insert(key, value);
        }
    }
}

fn parse_database(value: Option<&str>) -> Result<u32, DatabaseError> {
    value.unwrap_or("0").parse::<u32>().map_err(|_| {
        DatabaseError::configuration("Redis-compatible database must be a non-negative integer")
    })
}

fn normalized_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

fn url_host(host: &str) -> String {
    let host = normalized_host(host);
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

fn tunnel_unavailable() -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Network,
        code: Some("ssh_tunnel_unavailable".into()),
        message: "SSH tunnel transport is no longer available; reconnect the Redis profile".into(),
        diagnostic: None,
    }
}

fn encode_url_part(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn redis_error(error: redis::RedisError, default_category: ErrorCategory) -> DatabaseError {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    let tunnel_unavailable = is_tunnel_unavailable(&error);
    let category = if tunnel_unavailable {
        ErrorCategory::Network
    } else if lower.contains("wrongpass") || lower.contains("noauth") {
        ErrorCategory::Authentication
    } else if lower.contains("noperm") {
        ErrorCategory::Permission
    } else if lower.contains("moved") || lower.contains("ask") {
        ErrorCategory::Unsupported
    } else {
        default_category
    };
    DatabaseError {
        category,
        code: tunnel_unavailable
            .then(|| "ssh_tunnel_unavailable".to_owned())
            .or_else(|| error.code().map(str::to_owned)),
        message: crate::security::sanitize_terminal_text(&message),
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_url_host_formats_ipv6_literals_without_double_brackets() {
        assert_eq!(url_host("redis.internal"), "redis.internal");
        assert_eq!(url_host("192.0.2.10"), "192.0.2.10");
        assert_eq!(url_host("::1"), "[::1]");
        assert_eq!(url_host("[2001:db8::10]"), "[2001:db8::10]");
        assert_eq!(normalized_host("[2001:db8::10]"), "2001:db8::10");
    }

    #[test]
    fn redis_error_classifies_auth_permission_and_cluster_redirects() {
        let error = redis::RedisError::from((redis::ErrorKind::AuthenticationFailed, "WRONGPASS"));
        assert_eq!(
            redis_error(error, ErrorCategory::Network).category,
            ErrorCategory::Authentication
        );
        let error = redis::RedisError::from((redis::ErrorKind::InvalidClientConfig, "NOPERM"));
        assert_eq!(
            redis_error(error, ErrorCategory::Network).category,
            ErrorCategory::Permission
        );
    }

    #[test]
    fn tunnel_failure_is_network_not_authentication_when_a_password_is_present() {
        let error = redis::RedisError::from((
            redis::ErrorKind::Io,
            "SSH tunnel transport is no longer available; reconnect the Redis profile",
            "ssh_tunnel_unavailable".to_owned(),
        ));
        let mapped = redis_error(error, ErrorCategory::Authentication);
        assert_eq!(mapped.category, ErrorCategory::Network);
        assert_eq!(mapped.code.as_deref(), Some("ssh_tunnel_unavailable"));
    }
}
