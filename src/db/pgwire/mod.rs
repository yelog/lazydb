//! Shared PostgreSQL wire protocol mechanics; server-specific catalogs live in adapters.
pub(crate) mod decode;

use super::{
    DatabaseError, ErrorCategory,
    query::{QueryBudget, QueryOutcome, QueryOutcomeAccumulator},
};
use crate::profile::{ConnectionProfile, SslMode};
use futures_util::TryStreamExt;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    AssertSqlSafe, Connection, Either, PgPool, Postgres,
    pool::PoolConnection,
    postgres::{PgConnectOptions, PgConnection, PgRow, PgSslMode},
};

pub(crate) async fn connect_options(
    profile: &ConnectionProfile,
    password: Option<&SecretString>,
    default_port: u16,
) -> Result<PgConnectOptions, DatabaseError> {
    let host = profile
        .host
        .as_deref()
        .ok_or_else(|| DatabaseError::configuration("pgwire profile has no host"))?;
    // Parse only the startup `options` through SQLx's URL parser. No credentials enter this URL.
    let mut options = if let Some(raw) = profile
        .pg_options
        .as_ref()
        .and_then(|pg| pg.options.as_ref())
    {
        let mut url = url::Url::parse("postgresql://localhost").expect("static pgwire URL");
        url.query_pairs_mut().append_pair("options", raw);
        url.as_str()
            .parse::<PgConnectOptions>()
            .map_err(|_| DatabaseError::configuration("invalid PostgreSQL startup options"))?
    } else {
        PgConnectOptions::new()
    };
    options = options
        .host(host)
        .port(profile.port.unwrap_or(default_port))
        .ssl_mode(match profile.ssl_mode {
            SslMode::Disable => PgSslMode::Disable,
            SslMode::Prefer => PgSslMode::Prefer,
            SslMode::Require => PgSslMode::Require,
            SslMode::VerifyCa => PgSslMode::VerifyCa,
            SslMode::VerifyFull => PgSslMode::VerifyFull,
        })
        .application_name("lazydb");
    if let Some(user) = &profile.user {
        options = options.username(user);
    }
    if let Some(database) = &profile.database {
        options = options.database(database);
    }
    if let Some(password) = password {
        options = options.password(password.expose_secret());
    }
    if let Some(pg) = &profile.pg_options {
        if let Some(path) = &pg.ssl_root_cert {
            if matches!(profile.ssl_mode, SslMode::VerifyCa | SslMode::VerifyFull) {
                let pem = tokio::fs::read(path).await.map_err(|_| {
                    DatabaseError::configuration("unable to read PostgreSQL root certificate file")
                })?;
                validate_root_certificate(&pem)?;
                options = options.ssl_root_cert_from_pem(pem);
            } else {
                options = options.ssl_root_cert(path);
            }
        }
        if let Some(path) = &pg.ssl_client_cert {
            options = options.ssl_client_cert(path);
        }
        if let Some(path) = &pg.ssl_client_key {
            options = options.ssl_client_key(path);
        }
    }
    if profile.read_only {
        options = options.options([("default_transaction_read_only", "on")]);
    }
    if let Some(schema) = &profile.default_schema {
        let search_path = format!("\"{}\", public", schema.replace('"', "\"\""));
        options = options.options([("search_path", search_path.as_str())]);
    }
    Ok(options)
}

fn validate_root_certificate(pem: &[u8]) -> Result<(), DatabaseError> {
    let certificates = reqwest::Certificate::from_pem_bundle(pem)
        .map_err(|_| DatabaseError::configuration("invalid PostgreSQL root certificate PEM"))?;
    if certificates.is_empty() {
        return Err(DatabaseError::configuration(
            "PostgreSQL root certificate file contains no PEM certificates",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum Server {
    Postgres,
    Cockroach,
}

// SQLx normally drains an unfinished query before recycling a pooled connection.
// Aborting the caller must instead discard that session and interrupt server work.
struct ExecutingSession {
    connection: Option<PoolConnection<Postgres>>,
    options: std::sync::Arc<PgConnectOptions>,
    cancel_sql: Option<String>,
}

impl Drop for ExecutingSession {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        let Some(cancel_sql) = self.cancel_sql.take() else {
            drop(connection.detach());
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            drop(connection.detach());
            return;
        };
        let options = self.options.clone();
        // Retain the pool permit while cancellation is in flight, bounding control
        // connections by pool capacity. This guard discards the socket even when
        // the cancellation task itself is dropped during runtime shutdown.
        let disposal = Self {
            connection: Some(connection),
            options: options.clone(),
            cancel_sql: None,
        };
        runtime.spawn(async move {
            // Use a separate connection: every pool slot may be executing a query.
            // Keep the original socket alive until cancellation has been sent so its
            // backend/session ID cannot be recycled for another client meanwhile.
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut control = PgConnection::connect_with(&options).await?;
                let result = sqlx::raw_sql(AssertSqlSafe(cancel_sql.as_str()))
                    .execute(&mut control)
                    .await;
                drop(control);
                result
            })
            .await;
            // Socket disposal is unconditional, including failed cancellation. A
            // cancellation racing a commit cannot promise that writes rolled back.
            drop(disposal);
        });
    }
}

pub(crate) async fn execute_pool(
    pool: &PgPool,
    server: Server,
    sql: &str,
    budget: QueryBudget,
) -> Result<QueryOutcome, DatabaseError> {
    let mut session = ExecutingSession {
        connection: Some(
            pool.acquire()
                .await
                .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?,
        ),
        options: pool.connect_options(),
        cancel_sql: None,
    };
    let connection = session.connection.as_mut().expect("executing session");
    session.cancel_sql = Some(match server {
        Server::Postgres => {
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut **connection)
                .await
                .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
            format!("SELECT pg_cancel_backend({pid})")
        }
        Server::Cockroach => {
            let id: String = sqlx::query_scalar("SHOW session_id")
                .fetch_one(&mut **connection)
                .await
                .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
            format!("CANCEL SESSION IF EXISTS '{}'", id.replace('\'', "''"))
        }
    });
    let result = execute_connection(connection, sql, budget).await;
    session.cancel_sql = None;
    if result.is_ok() {
        // Fully drained results can safely return this session to its pool.
        drop(session.connection.take());
    }
    result
}

pub(crate) async fn execute_connection(
    connection: &mut PgConnection,
    sql: &str,
    budget: QueryBudget,
) -> Result<QueryOutcome, DatabaseError> {
    let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(connection);
    collect_stream(&mut stream, budget).await
}

async fn collect_stream<E>(
    stream: &mut E,
    budget: QueryBudget,
) -> Result<QueryOutcome, DatabaseError>
where
    E: futures_util::TryStream<
            Ok = Either<sqlx::postgres::PgQueryResult, PgRow>,
            Error = sqlx::Error,
        > + Unpin,
{
    let mut accumulator = QueryOutcomeAccumulator::with_budget(budget);
    while let Some(event) = stream
        .try_next()
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
    {
        match event {
            Either::Right(row) => accumulator.row(decode::columns(&row), decode::decode_row(&row)),
            Either::Left(done) => accumulator.done(done.rows_affected()),
        }
    }
    Ok(accumulator.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::import_connection_url;

    #[test]
    fn root_certificate_rejects_empty_text_and_malformed_pem() {
        for pem in [
            b"".as_slice(),
            b"not a certificate".as_slice(),
            b"-----BEGIN CERTIFICATE-----\n%%%\n-----END CERTIFICATE-----".as_slice(),
        ] {
            assert!(validate_root_certificate(pem).is_err());
        }
    }

    #[tokio::test]
    async fn startup_options_preserve_raw_routing_options_and_profile_overrides() {
        let mut imported = import_connection_url(
            "postgresql://alice@localhost:26257/defaultdb?sslmode=verify-full&options=--cluster%3Dtenant-1",
            Some("pgwire-options"),
        ).unwrap();
        imported.profile.read_only = true;
        imported.profile.default_schema = Some("quoted\"schema".into());
        let options = connect_options(&imported.profile, None, 26257)
            .await
            .unwrap();
        assert_eq!(options.get_username(), "alice");
        assert_eq!(options.get_port(), 26257);
        assert_eq!(options.get_database(), Some("defaultdb"));
        assert!(matches!(options.get_ssl_mode(), PgSslMode::VerifyFull));
        let startup = options.get_options().unwrap();
        assert!(startup.starts_with("--cluster=tenant-1"));
        assert!(startup.contains("default_transaction_read_only=on"));
        assert!(startup.contains("quoted\"\"schema"));
    }
}
