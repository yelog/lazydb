use std::{
    fmt,
    future::Future,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use redis::{
    AsyncConnectionConfig, Client, ErrorKind, RedisError, Value,
    aio::{ConnectionLike, ConnectionManager, MultiplexedConnection},
};
use tokio::sync::Mutex;

use super::reconnect::{self, ReconnectPolicy};
use crate::{db::transport::ConnectionRoute, ssh::TunnelLease};

pub(crate) const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
const TUNNEL_UNAVAILABLE_DETAIL: &str = "ssh_tunnel_unavailable";

type SharedConnect = Shared<BoxFuture<'static, redis::RedisResult<MultiplexedConnection>>>;

#[derive(Clone, Debug)]
pub(crate) enum RedisConnection {
    Direct(ConnectionManager),
    Routed(Arc<RoutedConnection>),
}

impl RedisConnection {
    pub(crate) async fn connect_routed(
        client: Client,
        route: ConnectionRoute,
        endpoint: SocketAddr,
        expected_host: &str,
        database: i64,
    ) -> redis::RedisResult<Self> {
        let config = AsyncConnectionConfig::new()
            .set_connection_timeout(Some(CONNECTION_TIMEOUT))
            .set_response_timeout(Some(RESPONSE_TIMEOUT))
            .set_dns_resolver(RouteResolver {
                expected_host: expected_host.to_owned(),
                endpoint,
            });
        let routed = Arc::new(RoutedConnection {
            client,
            route,
            config,
            policy: reconnect::policy(),
            database,
            expected_host: expected_host.to_owned(),
            endpoint,
            state: Mutex::new(RoutedState::default()),
        });
        routed.connection().await?;
        if let Some(lease) = routed.route.lease.as_ref() {
            let failure = lease.failure();
            let weak = Arc::downgrade(&routed);
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    failure.await;
                    if let Some(routed) = weak.upgrade() {
                        routed.invalidate_all().await;
                    }
                });
            }
        }
        Ok(Self::Routed(routed))
    }

    pub(crate) fn direct(manager: ConnectionManager) -> Self {
        Self::Direct(manager)
    }
}

impl ConnectionLike for RedisConnection {
    fn req_packed_command<'a>(
        &'a mut self,
        command: &'a redis::Cmd,
    ) -> futures_util::future::BoxFuture<'a, redis::RedisResult<Value>> {
        match self {
            Self::Direct(connection) => connection.req_packed_command(command),
            Self::Routed(connection) => {
                let connection = Arc::clone(connection);
                Box::pin(async move { connection.run_command(command).await })
            }
        }
    }

    fn req_packed_commands<'a>(
        &'a mut self,
        pipeline: &'a redis::Pipeline,
        offset: usize,
        count: usize,
    ) -> futures_util::future::BoxFuture<'a, redis::RedisResult<Vec<Value>>> {
        match self {
            Self::Direct(connection) => connection.req_packed_commands(pipeline, offset, count),
            Self::Routed(connection) => {
                let connection = Arc::clone(connection);
                Box::pin(async move { connection.run_pipeline(pipeline, offset, count).await })
            }
        }
    }

    fn get_db(&self) -> i64 {
        match self {
            Self::Direct(connection) => connection.get_db(),
            Self::Routed(connection) => connection.database,
        }
    }
}

pub(crate) struct RoutedConnection {
    client: Client,
    route: ConnectionRoute,
    config: AsyncConnectionConfig,
    policy: ReconnectPolicy,
    database: i64,
    expected_host: String,
    endpoint: SocketAddr,
    state: Mutex<RoutedState>,
}

impl fmt::Debug for RoutedConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RoutedConnection")
            .field("host", &self.expected_host)
            .field("endpoint", &self.endpoint)
            .field("database", &self.database)
            .field("route", &self.route)
            .finish_non_exhaustive()
    }
}

impl RoutedConnection {
    async fn run_command(&self, command: &redis::Cmd) -> redis::RedisResult<Value> {
        let (mut client, generation) = self.connection().await?;
        let result = self
            .guard_lease(async { client.req_packed_command(command).await })
            .await;
        self.invalidate_if_broken(generation, result.as_ref().err())
            .await;
        result
    }

    async fn run_pipeline(
        &self,
        pipeline: &redis::Pipeline,
        offset: usize,
        count: usize,
    ) -> redis::RedisResult<Vec<Value>> {
        let (mut client, generation) = self.connection().await?;
        let result = self
            .guard_lease(async { client.req_packed_commands(pipeline, offset, count).await })
            .await;
        self.invalidate_if_broken(generation, result.as_ref().err())
            .await;
        result
    }

    async fn connection(&self) -> redis::RedisResult<(MultiplexedConnection, u64)> {
        let attempt = {
            let mut state = self.state.lock().await;
            self.ensure_lease_alive()?;
            if let Some(connection) = state.connection.as_ref() {
                return Ok((connection.clone(), state.generation));
            }
            state
                .connecting
                .get_or_insert_with(|| self.shared_connect())
                .clone()
        };
        let result = attempt.clone().await;
        let mut state = self.state.lock().await;
        match result {
            Ok(connection) => {
                self.ensure_lease_alive()?;
                state.clear_connecting(&attempt);
                if state.connection.is_none() {
                    let generation = state.next_generation();
                    state.install(connection, generation);
                }
                let connection = state
                    .connection
                    .as_ref()
                    .expect("a routed connection was just installed")
                    .clone();
                Ok((connection, state.generation))
            }
            Err(error) => {
                state.clear_connecting(&attempt);
                Err(error)
            }
        }
    }

    fn shared_connect(&self) -> SharedConnect {
        let client = self.client.clone();
        let config = self.config.clone();
        let policy = self.policy;
        let route = self.route.clone();
        async move { connect_with_retries(client, config, policy, route).await }
            .boxed()
            .shared()
    }

    async fn guard_lease<T>(
        &self,
        future: impl Future<Output = redis::RedisResult<T>>,
    ) -> redis::RedisResult<T> {
        let Some(lease) = self.route.lease.as_ref() else {
            return future.await;
        };
        let failure = lease.failure();
        self.ensure_lease_alive()?;
        tokio::select! {
            biased;
            () = failure => Err(tunnel_unavailable_error()),
            result = future => {
                self.ensure_lease_alive()?;
                result
            },
        }
    }

    async fn invalidate_if_broken(&self, generation: u64, error: Option<&RedisError>) {
        if error.is_some_and(RedisError::is_connection_dropped) || !self.lease_is_alive() {
            self.invalidate(generation).await;
        }
    }

    async fn invalidate(&self, generation: u64) {
        self.state.lock().await.invalidate(generation);
    }

    async fn invalidate_all(&self) {
        self.state.lock().await.invalidate_all();
    }

    fn ensure_lease_alive(&self) -> redis::RedisResult<()> {
        if self.lease_is_alive() {
            Ok(())
        } else {
            Err(tunnel_unavailable_error())
        }
    }

    fn lease_is_alive(&self) -> bool {
        self.route.lease.as_ref().is_none_or(TunnelLease::is_alive)
    }
}

#[derive(Default)]
struct RoutedState {
    generation: u64,
    connection: Option<MultiplexedConnection>,
    connecting: Option<SharedConnect>,
}

impl RoutedState {
    fn install(&mut self, connection: MultiplexedConnection, generation: u64) {
        self.generation = generation;
        self.connection = Some(connection);
    }

    fn next_generation(&mut self) -> u64 {
        self.generation.wrapping_add(1)
    }

    fn invalidate(&mut self, generation: u64) {
        if self.generation == generation {
            self.connection = None;
            self.connecting = None;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn invalidate_all(&mut self) {
        self.connection = None;
        self.connecting = None;
        self.generation = self.generation.wrapping_add(1);
    }

    fn clear_connecting(&mut self, attempt: &SharedConnect) {
        if self
            .connecting
            .as_ref()
            .is_some_and(|connecting| connecting.ptr_eq(attempt))
        {
            self.connecting = None;
        }
    }
}

async fn connect_with_retries(
    client: Client,
    config: AsyncConnectionConfig,
    policy: ReconnectPolicy,
    route: ConnectionRoute,
) -> redis::RedisResult<MultiplexedConnection> {
    let mut last_error = None;
    for attempt in 0..=policy.attempts {
        ensure_route_alive(&route)?;
        if attempt > 0 {
            wait_for_retry(policy.delay_for(attempt - 1).min(policy.max_delay), &route).await?;
        }
        let result = guard_route(
            &route,
            client.get_multiplexed_async_connection_with_config(&config),
        )
        .await;
        match result {
            Ok(connection) => {
                ensure_route_alive(&route)?;
                return Ok(connection);
            }
            Err(error) => {
                ensure_route_alive(&route)?;
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(tunnel_unavailable_error))
}

async fn wait_for_retry(delay: Duration, route: &ConnectionRoute) -> redis::RedisResult<()> {
    let Some(lease) = route.lease.as_ref() else {
        tokio::time::sleep(delay).await;
        return Ok(());
    };
    let failure = lease.failure();
    ensure_route_alive(route)?;
    tokio::select! {
        biased;
        () = failure => Err(tunnel_unavailable_error()),
        () = tokio::time::sleep(delay) => ensure_route_alive(route),
    }
}

async fn guard_route<T>(
    route: &ConnectionRoute,
    future: impl Future<Output = redis::RedisResult<T>>,
) -> redis::RedisResult<T> {
    let Some(lease) = route.lease.as_ref() else {
        return future.await;
    };
    let failure = lease.failure();
    ensure_route_alive(route)?;
    tokio::select! {
        biased;
        () = failure => Err(tunnel_unavailable_error()),
        result = future => {
            ensure_route_alive(route)?;
            result
        },
    }
}

fn ensure_route_alive(route: &ConnectionRoute) -> redis::RedisResult<()> {
    if route.lease.as_ref().is_none_or(TunnelLease::is_alive) {
        Ok(())
    } else {
        Err(tunnel_unavailable_error())
    }
}

struct RouteResolver {
    expected_host: String,
    endpoint: SocketAddr,
}

impl redis::io::AsyncDNSResolver for RouteResolver {
    fn resolve<'a, 'b: 'a>(
        &'a self,
        host: &'b str,
        _port: u16,
    ) -> redis::RedisFuture<'a, Box<dyn Iterator<Item = SocketAddr> + Send + 'a>> {
        let matched = host.eq_ignore_ascii_case(&self.expected_host)
            || match (host.parse::<IpAddr>(), self.expected_host.parse::<IpAddr>()) {
                (Ok(requested), Ok(expected)) => requested == expected,
                _ => false,
            };
        let endpoint = self.endpoint;
        Box::pin(async move {
            if matched {
                Ok(Box::new(std::iter::once(endpoint))
                    as Box<dyn Iterator<Item = SocketAddr> + Send>)
            } else {
                Err(RedisError::from((
                    ErrorKind::InvalidClientConfig,
                    "Redis routed DNS request did not match the profile host",
                )))
            }
        })
    }
}

fn tunnel_unavailable_error() -> RedisError {
    RedisError::from((
        ErrorKind::Io,
        "SSH tunnel transport is no longer available; reconnect the Redis profile",
        TUNNEL_UNAVAILABLE_DETAIL.to_owned(),
    ))
}

pub(crate) fn is_tunnel_unavailable(error: &RedisError) -> bool {
    error.detail() == Some(TUNNEL_UNAVAILABLE_DETAIL)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use redis::io::AsyncDNSResolver;
    use secrecy::SecretString;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use super::*;
    use crate::{
        db::{ErrorCategory, redis::RedisAdapter, transport::TransportEndpoint},
        profile::{ConnectionProfile, SshOptions, import_connection_url},
    };

    #[derive(Default)]
    struct ServerState {
        connections: AtomicUsize,
        pings: AtomicUsize,
        deletes: AtomicUsize,
        drop_next_delete: AtomicBool,
        reject_new: AtomicBool,
        auth: StdMutex<Vec<Vec<Vec<u8>>>>,
        select: StdMutex<Vec<Vec<u8>>>,
    }

    async fn start_server(drop_next_delete: bool) -> (SocketAddr, Arc<ServerState>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(ServerState {
            drop_next_delete: AtomicBool::new(drop_next_delete),
            ..ServerState::default()
        });
        let server_state = Arc::clone(&state);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve_connection(stream, Arc::clone(&server_state)));
            }
        });
        (address, state)
    }

    async fn serve_connection(mut stream: TcpStream, state: Arc<ServerState>) {
        state.connections.fetch_add(1, Ordering::SeqCst);
        if state.reject_new.load(Ordering::SeqCst) {
            return;
        }
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let count = match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(count) => count,
            };
            buffer.extend_from_slice(&chunk[..count]);
            while let Some((end, arguments)) = parse_request(&buffer) {
                buffer.drain(..end);
                let Some(command) = arguments.first() else {
                    continue;
                };
                if command.eq_ignore_ascii_case(b"AUTH") {
                    state.auth.lock().unwrap().push(arguments);
                    stream.write_all(b"+OK\r\n").await.unwrap();
                } else if command.eq_ignore_ascii_case(b"SELECT") {
                    state
                        .select
                        .lock()
                        .unwrap()
                        .push(arguments.get(1).cloned().unwrap_or_default());
                    stream.write_all(b"+OK\r\n").await.unwrap();
                } else if command.eq_ignore_ascii_case(b"CLIENT") {
                    stream.write_all(b"+OK\r\n").await.unwrap();
                } else if command.eq_ignore_ascii_case(b"PING") {
                    state.pings.fetch_add(1, Ordering::SeqCst);
                    stream.write_all(b"+PONG\r\n").await.unwrap();
                } else if command.eq_ignore_ascii_case(b"DEL") {
                    state.deletes.fetch_add(1, Ordering::SeqCst);
                    if state.drop_next_delete.swap(false, Ordering::SeqCst) {
                        return;
                    }
                    stream.write_all(b":1\r\n").await.unwrap();
                } else {
                    stream.write_all(b"+OK\r\n").await.unwrap();
                }
            }
        }
    }

    fn parse_request(buffer: &[u8]) -> Option<(usize, Vec<Vec<u8>>)> {
        if buffer.first().copied()? != b'*' {
            return None;
        }
        let line_end = buffer.windows(2).position(|window| window == b"\r\n")?;
        let count = std::str::from_utf8(&buffer[1..line_end])
            .ok()?
            .parse::<usize>()
            .ok()?;
        let mut position = line_end + 2;
        let mut arguments = Vec::with_capacity(count);
        for _ in 0..count {
            if buffer.get(position).copied()? != b'$' {
                return None;
            }
            let length_end = position
                + 1
                + buffer[position + 1..]
                    .windows(2)
                    .position(|window| window == b"\r\n")?;
            let length = std::str::from_utf8(&buffer[position + 1..length_end])
                .ok()?
                .parse::<usize>()
                .ok()?;
            let value_start = length_end + 2;
            let value_end = value_start.checked_add(length)?;
            if buffer.get(value_end..value_end + 2)? != b"\r\n" {
                return None;
            }
            arguments.push(buffer[value_start..value_end].to_vec());
            position = value_end + 2;
        }
        Some((position, arguments))
    }

    fn routed_route(address: SocketAddr) -> ConnectionRoute {
        ConnectionRoute {
            endpoint: TransportEndpoint::Tcp(address),
            lease: None,
        }
    }

    fn profile(url: &str) -> (ConnectionProfile, Option<SecretString>) {
        let imported = import_connection_url(url, Some("routed-redis-test")).unwrap();
        (imported.profile, imported.transient_password)
    }

    #[tokio::test]
    async fn routed_connection_keeps_auth_select_and_reconnects_once_without_replaying_delete() {
        let (address, state) = start_server(true).await;
        let (profile, password) = profile("redis://app:test-password@redis.routed.invalid:6379/2");
        let adapter =
            RedisAdapter::connect_routed(&profile, password.as_ref(), routed_route(address))
                .await
                .unwrap();

        assert_eq!(adapter.database(), 2);
        assert_eq!(
            state.auth.lock().unwrap().as_slice(),
            &[vec![
                b"AUTH".to_vec(),
                b"app".to_vec(),
                b"test-password".to_vec()
            ]]
        );
        assert_eq!(state.select.lock().unwrap().as_slice(), &[b"2".to_vec()]);

        let mut connection = adapter.connection_clone();
        let pongs = redis::pipe()
            .cmd("PING")
            .cmd("PING")
            .query_async::<Vec<String>>(&mut connection)
            .await
            .unwrap();
        assert_eq!(pongs, vec!["PONG", "PONG"]);

        let error = adapter.delete_key(b"write-once").await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Network);
        assert_eq!(state.deletes.load(Ordering::SeqCst), 1);

        let adapter = Arc::new(adapter);
        let mut pings = Vec::new();
        for _ in 0..20 {
            let adapter = Arc::clone(&adapter);
            pings.push(tokio::spawn(async move { adapter.ping().await.unwrap() }));
        }
        for ping in pings {
            ping.await.unwrap();
        }

        assert_eq!(state.connections.load(Ordering::SeqCst), 2);
        assert_eq!(state.deletes.load(Ordering::SeqCst), 1);
        assert_eq!(state.auth.lock().unwrap().len(), 2);
        assert_eq!(
            state.select.lock().unwrap().as_slice(),
            &[b"2".to_vec(), b"2".to_vec()]
        );
        assert!(state.pings.load(Ordering::SeqCst) >= 21);
    }

    #[tokio::test]
    async fn concurrent_commands_share_one_bounded_failed_reconnect_cycle() {
        let (address, state) = start_server(true).await;
        let (profile, password) = profile("redis://redis.routed.invalid:6379/0");
        let adapter =
            RedisAdapter::connect_routed(&profile, password.as_ref(), routed_route(address))
                .await
                .unwrap();

        assert!(adapter.delete_key(b"drop-the-connection").await.is_err());
        state.reject_new.store(true, Ordering::SeqCst);
        let adapter = Arc::new(adapter);
        let mut pings = Vec::new();
        for _ in 0..20 {
            let adapter = Arc::clone(&adapter);
            pings.push(tokio::spawn(async move { adapter.ping().await }));
        }
        let results = tokio::time::timeout(Duration::from_secs(5), async {
            let mut results = Vec::new();
            for ping in pings {
                results.push(ping.await.unwrap());
            }
            results
        })
        .await
        .expect("a failed reconnect cycle must respect the bounded retry policy");

        assert!(results.iter().all(Result::is_err));
        assert_eq!(state.connections.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn routed_ip_literal_uses_the_route_endpoint_for_a_successful_connection() {
        let (address, state) = start_server(false).await;
        let (profile, password) = profile("redis://192.0.2.10:6379/2");
        let adapter =
            RedisAdapter::connect_routed(&profile, password.as_ref(), routed_route(address))
                .await
                .unwrap();

        assert_eq!(adapter.database(), 2);
        adapter.ping().await.unwrap();
        assert_eq!(state.connections.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn routed_ip_literal_maps_to_the_route_and_never_dials_the_logical_address() {
        let (direct_address, direct_state) = start_server(false).await;
        let route_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let route_address = route_listener.local_addr().unwrap();
        drop(route_listener);
        let (profile, password) = profile(&format!("redis://{direct_address}/0"));

        let error = tokio::time::timeout(
            Duration::from_secs(8),
            RedisAdapter::connect_routed(&profile, password.as_ref(), routed_route(route_address)),
        )
        .await
        .unwrap()
        .unwrap_err();

        assert_eq!(error.category, ErrorCategory::Network);
        assert_eq!(direct_state.connections.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn routed_redis_rejects_unsupported_transport_endpoints() {
        let (profile, password) = profile("redis://redis.routed.invalid:6379/0");
        for endpoint in [
            TransportEndpoint::Unix(std::path::PathBuf::from("/tmp/redis.sock")),
            TransportEndpoint::Socks5("127.0.0.1:1080".parse().unwrap()),
        ] {
            let error = RedisAdapter::connect_routed(
                &profile,
                password.as_ref(),
                ConnectionRoute {
                    endpoint,
                    lease: None,
                },
            )
            .await
            .unwrap_err();
            assert_eq!(error.category, ErrorCategory::Unsupported);
            assert!(error.message.contains("do not support"));
        }
    }

    #[tokio::test]
    async fn direct_redis_rejects_an_ssh_profile_before_any_dial() {
        let (address, state) = start_server(false).await;
        let (mut profile, password) = profile(&format!("redis://{address}/0"));
        profile.ssh = Some(SshOptions {
            host: "bastion.invalid".into(),
            ..SshOptions::default()
        });

        let error = RedisAdapter::connect(&profile, password.as_ref())
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert_eq!(state.connections.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn route_resolver_maps_only_the_configured_domain_or_ip() {
        let endpoint: SocketAddr = "127.0.0.1:46379".parse().unwrap();
        let domain = RouteResolver {
            expected_host: "redis.internal".into(),
            endpoint,
        };
        assert_eq!(
            domain
                .resolve("REDIS.internal", 6379)
                .await
                .unwrap()
                .collect::<Vec<_>>(),
            vec![endpoint]
        );
        assert!(domain.resolve("other.internal", 6379).await.is_err());

        let ip = RouteResolver {
            expected_host: "192.0.2.10".into(),
            endpoint,
        };
        assert_eq!(
            ip.resolve("192.0.2.10", 6379)
                .await
                .unwrap()
                .collect::<Vec<_>>(),
            vec![endpoint]
        );
        assert!(ip.resolve("192.0.2.11", 6379).await.is_err());
    }
}
