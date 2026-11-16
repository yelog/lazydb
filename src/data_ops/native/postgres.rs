use std::{path::Path, process::Stdio};

use anyhow::{Context, Result, bail, ensure};
use secrecy::ExposeSecret;
use tokio_util::sync::CancellationToken;

use crate::{
    connections::DatabaseConnector,
    data_ops::{
        DataJobSpec, DataJobStatus, DataOperation, NativeOutcome, NativePlan, NativeToolVersion,
        ProgressSink, ResolvedEndpoint,
    },
    db::{
        DatabaseConnection,
        transport::{ConnectionRoute, TransportEndpoint},
    },
    profile::{DatabaseKind, SslMode},
};

use super::common::{self, PinnedInput, Process, Redactor, StagedArtifact};

fn conninfo_value(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn connection_string(
    endpoint: &ResolvedEndpoint,
    route: &ConnectionRoute,
    client_major: u32,
) -> Result<String> {
    let profile = &endpoint.profile;
    let host = profile
        .host
        .as_deref()
        .context("PostgreSQL host is missing")?;
    let user = profile
        .user
        .as_deref()
        .context("PostgreSQL user is missing")?;
    let mut parameters = vec![
        ("host", host.to_owned()),
        ("port", profile.port.unwrap_or(5432).to_string()),
        ("user", user.to_owned()),
        ("dbname", endpoint.database.clone()),
        ("connect_timeout", "10".into()),
        ("application_name", "lazydb-data".into()),
        ("gssencmode", "disable".into()),
        (
            "sslmode",
            match profile.ssl_mode {
                SslMode::Disable => "disable",
                SslMode::Prefer => "prefer",
                SslMode::Require => "require",
                SslMode::VerifyCa => "verify-ca",
                SslMode::VerifyFull => "verify-full",
            }
            .into(),
        ),
    ];
    match &route.endpoint {
        TransportEndpoint::Direct => {}
        TransportEndpoint::Tcp(address) => {
            parameters.push(("hostaddr", address.ip().to_string()));
            parameters.retain(|(name, _)| *name != "port");
            parameters.push(("port", address.port().to_string()));
        }
        TransportEndpoint::Unix(_) | TransportEndpoint::Socks5(_) => {
            bail!("native PostgreSQL requires a direct or managed TCP route to preserve TLS")
        }
    }
    let mut options = String::new();
    if let Some(pg) = &profile.pg_options {
        for (name, path) in [
            ("sslrootcert", pg.ssl_root_cert.as_ref()),
            ("sslcert", pg.ssl_client_cert.as_ref()),
            ("sslkey", pg.ssl_client_key.as_ref()),
        ] {
            if let Some(path) = path {
                let absolute = std::fs::canonicalize(path)
                    .context("PostgreSQL TLS certificate/key file is unavailable")?;
                ensure!(
                    absolute.is_file(),
                    "PostgreSQL TLS certificate/key must be a file"
                );
                parameters.push((name, common::path_text(&absolute)?.to_owned()));
            }
        }
        if let Some(value) = &pg.options {
            common::clean_text(value)?;
            options.push_str(value);
        }
    }
    if matches!(profile.ssl_mode, SslMode::VerifyCa | SslMode::VerifyFull)
        && profile
            .pg_options
            .as_ref()
            .and_then(|options| options.ssl_root_cert.as_ref())
            .is_none()
    {
        ensure!(
            profile.ssl_mode == SslMode::VerifyFull && client_major >= 16,
            "native PostgreSQL TLS verification needs an explicit root certificate, or pg_dump/pg_restore 16+ with verify-full and system roots"
        );
        parameters.push(("sslrootcert", "system".into()));
    }
    if profile.read_only {
        options.push_str(" -c default_transaction_read_only=on");
    }
    if !options.is_empty() {
        parameters.push(("options", options));
    }
    for (_, value) in &parameters {
        common::clean_text(value)?;
    }
    Ok(parameters
        .into_iter()
        .map(|(name, value)| format!("{name}={}", conninfo_value(&value)))
        .collect::<Vec<_>>()
        .join(" "))
}

fn client(
    tool: &NativeToolVersion,
    endpoint: &ResolvedEndpoint,
    route: &ConnectionRoute,
) -> Result<Process> {
    let mut process = Process::new(&tool.path)?;
    let password = endpoint
        .password
        .as_ref()
        .map(ExposeSecret::expose_secret)
        .unwrap_or("");
    ensure!(
        !password.contains(['\n', '\r', '\0']),
        "PostgreSQL passfiles cannot represent passwords containing line breaks or NUL"
    );
    let escaped = password.replace('\\', "\\\\").replace(':', "\\:");
    let passfile = process.private_file("pgpass", &format!("*:*:*:*:{escaped}\n"))?;
    process
        .command
        .env("PGPASSFILE", passfile)
        .arg("--no-password")
        .arg(format!(
            "--dbname={}",
            connection_string(endpoint, route, common::version_number(&tool.version)?.0)?
        ));
    Ok(process)
}

async fn route(
    connector: &DatabaseConnector,
    endpoint: &ResolvedEndpoint,
    cancellation: &CancellationToken,
) -> Result<ConnectionRoute> {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => bail!("data operation cancelled"),
        route = connector.native_tcp_route(&endpoint.profile) => route.context("cannot acquire native PostgreSQL route"),
    }
}

async fn source_version(
    endpoint: &ResolvedEndpoint,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<u32> {
    let connection =
        common::connect(connector, endpoint, Some(&endpoint.database), cancellation).await?;
    let result = async {
        let actual = common::scalar(
            &common::sql(&connection, "SELECT current_database()", cancellation).await?,
        )?;
        ensure!(
            actual == endpoint.database,
            "PostgreSQL source connection selected a different database"
        );
        let version =
            common::scalar(&common::sql(&connection, "SHOW server_version", cancellation).await?)?;
        Ok(common::version_number(&version)?.0)
    }
    .await;
    connection.close().await;
    result
}

async fn destination_connection(
    endpoint: &ResolvedEndpoint,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<DatabaseConnection> {
    match common::connect(connector, endpoint, Some("postgres"), cancellation).await {
        Ok(connection) => Ok(connection),
        Err(error) => {
            common::check_cancel(cancellation)?;
            common::connect(connector, endpoint, Some("template1"), cancellation)
                .await
                .with_context(|| format!("cannot open PostgreSQL maintenance database ({error})"))
        }
    }
}

async fn ensure_destination_absent(
    connection: &DatabaseConnection,
    database: &str,
    cancellation: &CancellationToken,
) -> Result<()> {
    let statement = format!(
        "SELECT count(*) FROM pg_database WHERE datname = {}",
        common::pg_literal(database)
    );
    let count = common::scalar(&common::sql(connection, &statement, cancellation).await?)?;
    ensure!(
        count == "0",
        "PostgreSQL destination database already exists"
    );
    Ok(())
}

async fn destination_version(
    endpoint: &ResolvedEndpoint,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<u32> {
    let connection = destination_connection(endpoint, connector, cancellation).await?;
    let result = async {
        ensure_destination_absent(&connection, &endpoint.database, cancellation).await?;
        let version =
            common::scalar(&common::sql(&connection, "SHOW server_version", cancellation).await?)?;
        Ok(common::version_number(&version)?.0)
    }
    .await;
    connection.close().await;
    result
}

async fn archive(
    path: &Path,
    tool: &NativeToolVersion,
    cancellation: &CancellationToken,
    redactor: &Redactor,
) -> Result<u32> {
    common::signature(path, b"PGDMP")?;
    let mut process = Process::new(&tool.path)?;
    process
        .command
        .arg("--list")
        .stdin(Stdio::from(common::reader(path)?));
    let output = process.run(cancellation, redactor, None, None).await?;
    let dumped_by = output
        .stdout
        .lines()
        .find_map(|line| {
            line.split_once("Dumped by pg_dump version:")
                .map(|(_, version)| version.trim())
        })
        .context("PostgreSQL archive list is missing the pg_dump version")?;
    let dumped_from = output
        .stdout
        .lines()
        .find_map(|line| {
            line.split_once("Dumped from database version:")
                .map(|(_, version)| version.trim())
        })
        .context("PostgreSQL archive list is missing the source server version")?;
    let dump_major = common::version_number(dumped_by)?.0;
    let source_major = common::version_number(dumped_from)?.0;
    ensure!(
        dump_major >= source_major,
        "PostgreSQL archive has incompatible producer/server versions"
    );
    ensure!(
        common::version_number(&tool.version)?.0 >= dump_major,
        "pg_restore is older than the archive producer"
    );
    Ok(dump_major)
}

pub(super) async fn plan(
    spec: &DataJobSpec,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<NativePlan> {
    let redactor = Redactor::new(&[source, destination]);
    let mut tools = Vec::new();
    if spec.operation != DataOperation::Restore {
        tools.push(
            common::tool(
                "pg_dump",
                spec.tools.dump.as_deref(),
                cancellation,
                &redactor,
            )
            .await?,
        );
    }
    tools.push(
        common::tool(
            "pg_restore",
            spec.tools.restore.as_deref(),
            cancellation,
            &redactor,
        )
        .await?,
    );
    for tool in &tools {
        ensure!(
            tool.version
                .starts_with(&format!("{} (PostgreSQL)", tool.name)),
            "native PostgreSQL executable reported an unexpected tool identity"
        );
        ensure!(
            common::version_number(&tool.version)?.0 >= 12,
            "PostgreSQL native clients must be version 12 or newer"
        );
    }
    let restore = tools
        .iter()
        .find(|tool| tool.name == "pg_restore")
        .context("pg_restore is missing")?;
    let dump_major = if let Some(source) = source {
        let dump = tools
            .iter()
            .find(|tool| tool.name == "pg_dump")
            .context("pg_dump is missing")?;
        let source_major = source_version(source, connector, cancellation).await?;
        let dump_major = common::version_number(&dump.version)?.0;
        ensure!(
            dump_major >= source_major,
            "pg_dump is older than the source PostgreSQL server"
        );
        ensure!(
            common::version_number(&restore.version)?.0 == dump_major,
            "pg_dump and pg_restore must use the same major version"
        );
        let route = route(connector, source, cancellation).await?;
        let _ = client(dump, source, &route)?;
        dump_major
    } else {
        archive(&spec.artifact, restore, cancellation, &redactor).await?
    };
    if let Some(destination) = destination {
        let destination_major = destination_version(destination, connector, cancellation).await?;
        ensure!(
            destination_major >= dump_major,
            "destination PostgreSQL server is older than the dump producer; downgrade restores are unsupported"
        );
        let route = route(connector, destination, cancellation).await?;
        let _ = client(restore, destination, &route)?;
    }
    let steps = match spec.operation {
        DataOperation::Backup => vec![
            "Dump the full PostgreSQL database using pg_dump --format=custom".into(),
            "Validate the custom archive using pg_restore --list, hash, and publish exclusively".into(),
        ],
        DataOperation::Restore => vec![
            "Validate and pin the PostgreSQL custom archive before creating a target".into(),
            "Recheck target absence and CREATE DATABASE using template0 without IF NOT EXISTS".into(),
            "Run pg_restore --exit-on-error --no-owner --no-privileges --no-tablespaces into the new target".into(),
        ],
        DataOperation::Copy => vec![
            "Create, validate, hash, and exclusively publish a PostgreSQL custom archive".into(),
            "Recheck target absence and create a new database using template0".into(),
            "Run portable pg_restore and keep the completed archive if restoration fails".into(),
        ],
    };
    let mut warnings = vec![
        "Archive verification means pg_restore --list can read the archive; it does not validate every data block or prove data parity.".into(),
        "Database-level dumps exclude cluster roles and other databases; required extensions and roles must be available on the destination.".into(),
    ];
    if spec.operation.writes_database() {
        warnings.extend([
            "Portable PostgreSQL restore uses --no-owner --no-privileges --no-tablespaces: original ownership, grants, and tablespace placement are not preserved.".into(),
            "The new PostgreSQL database uses the destination server's default encoding and locale; restore command success is not full data-parity verification.".into(),
            "Failure or cancellation after CREATE DATABASE leaves a possibly partial new database for manual inspection; it is never dropped automatically.".into(),
            "Restore only trusted native archives: stored definitions and extension SQL are executed by PostgreSQL.".into(),
        ]);
    }
    Ok(NativePlan {
        engine: DatabaseKind::Postgres,
        steps,
        warnings,
        tools,
    })
}

pub(super) async fn execute(
    spec: &DataJobSpec,
    plan: &NativePlan,
    endpoints: (Option<&ResolvedEndpoint>, Option<&ResolvedEndpoint>),
    pinned: Option<&PinnedInput>,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
    progress: &ProgressSink,
) -> Result<NativeOutcome> {
    let (source, destination) = endpoints;
    let redactor = Redactor::new(&[source, destination]);
    let restore = plan
        .tools
        .iter()
        .find(|tool| tool.name == "pg_restore")
        .context("pg_restore is missing")?;
    let mut outcome = NativeOutcome {
        warnings: plan.warnings.clone(),
        ..NativeOutcome::default()
    };
    let artifact = if let Some(source) = source {
        let output = common::output_path(&spec.artifact)?;
        let stage = StagedArtifact::new(&output)?;
        let partial = stage.path().to_path_buf();
        let dumped = async {
            let dump = plan
                .tools
                .iter()
                .find(|tool| tool.name == "pg_dump")
                .context("pg_dump is missing")?;
            let route = route(connector, source, cancellation).await?;
            let mut process = client(dump, source, &route)?;
            process
                .command
                .arg("--format=custom")
                .arg("--quote-all-identifiers")
                .stdout(Stdio::from(stage.writer()?));
            common::progress(
                progress,
                DataJobStatus::Running,
                "Running pg_dump custom archive backup",
                None,
            );
            let result = process
                .run(cancellation, &redactor, Some(&route), None)
                .await?;
            common::progress(
                progress,
                DataJobStatus::Verifying,
                "Validating PostgreSQL archive list",
                None,
            );
            archive(stage.path(), restore, cancellation, &redactor).await?;
            let fingerprint = common::fingerprint(stage.path(), cancellation).await?;
            common::check_cancel(cancellation)?;
            stage.publish(&output)?;
            Ok::<_, anyhow::Error>((fingerprint, result.stderr))
        }
        .await
        .with_context(|| {
            format!(
                "PostgreSQL backup failed; incomplete artifact retained at {}",
                partial.display()
            )
        })?;
        outcome.bytes = Some(dumped.0.0);
        outcome.sha256 = Some(dumped.0.1);
        outcome.artifact = Some(output.clone());
        outcome
            .verification
            .push("PostgreSQL custom archive is readable by pg_restore --list".into());
        if !dumped.1.is_empty() {
            outcome
                .warnings
                .push(format!("pg_dump diagnostics: {}", dumped.1));
        }
        common::backup_completed(progress, &outcome);
        output
    } else {
        let input = pinned.context("PostgreSQL restore input was not pinned")?;
        archive(input.path(), restore, cancellation, &redactor).await?;
        outcome.bytes = Some(input.bytes);
        outcome.sha256 = Some(input.sha256.clone());
        outcome.artifact = Some(spec.artifact.clone());
        outcome
            .verification
            .push("PostgreSQL input custom archive is readable by pg_restore --list".into());
        input.path().to_path_buf()
    };
    if spec.operation.writes_database() {
        common::check_cancel(cancellation)?;
        let destination = destination.context("PostgreSQL restore destination is missing")?;
        let restore_result = async {
            let copy_input = if spec.operation == DataOperation::Copy {
                let input = PinnedInput::new(&artifact, cancellation).await?;
                ensure!(
                    Some(input.bytes) == outcome.bytes
                        && Some(&input.sha256) == outcome.sha256.as_ref(),
                    "completed PostgreSQL backup changed before restore"
                );
                Some(input)
            } else {
                None
            };
            let restore_path = copy_input
                .as_ref()
                .map_or(artifact.as_path(), PinnedInput::path);
            let archive_major = archive(restore_path, restore, cancellation, &redactor).await?;
            let route = route(connector, destination, cancellation).await?;
            let mut process = client(restore, destination, &route)?;
            process
                .command
                .args([
                    "--exit-on-error",
                    "--no-owner",
                    "--no-privileges",
                    "--no-tablespaces",
                ])
                .stdin(Stdio::from(common::reader(restore_path)?));
            let connection = destination_connection(destination, connector, cancellation).await?;
            let creation = async {
                ensure_destination_absent(&connection, &destination.database, cancellation).await?;
                let version = common::scalar(
                    &common::sql(&connection, "SHOW server_version", cancellation).await?,
                )?;
                ensure!(
                    common::version_number(&version)?.0 >= archive_major,
                    "destination PostgreSQL server is older than the archive producer"
                );
                common::check_cancel(cancellation)?;
                common::progress(
                    progress,
                    DataJobStatus::Running,
                    "Creating new PostgreSQL database",
                    outcome.bytes,
                );
                let statement = format!(
                    "CREATE DATABASE {} TEMPLATE template0",
                    common::quote_identifier(&destination.database, false)?
                );
                common::sql(&connection, &statement, cancellation).await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            connection.close().await;
            creation?;
            outcome.target_created = true;
            common::progress(
                progress,
                DataJobStatus::Running,
                "Restoring PostgreSQL archive into the new database",
                outcome.bytes,
            );
            let result = process
                .run(cancellation, &redactor, Some(&route), None)
                .await?;
            common::progress(
                progress,
                DataJobStatus::Verifying,
                "PostgreSQL restore command completed; data parity was not checked",
                outcome.bytes,
            );
            if !result.stderr.is_empty() {
                outcome
                    .warnings
                    .push(format!("pg_restore diagnostics: {}", result.stderr));
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        restore_result.with_context(|| format!(
            "PostgreSQL restore failed; the new database may be partial and was not dropped; completed artifact kept at {}",
            spec.artifact.display()
        ))?;
        outcome.verification.push("pg_restore completed successfully with --exit-on-error; full data parity was not checked".into());
    }
    common::check_cancel(cancellation)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::import_connection_url;

    #[test]
    fn libpq_values_cannot_inject_parameters_or_replace_tls_over_a_tunnel() {
        let mut profile =
            import_connection_url("postgresql://user@db.example/test?sslmode=require", None)
                .unwrap()
                .profile;
        profile.database = Some("ignored".into());
        let endpoint = ResolvedEndpoint {
            profile,
            database: "a' host=attacker \\".into(),
            password: None,
        };
        let route = ConnectionRoute {
            endpoint: TransportEndpoint::Tcp("127.0.0.1:4567".parse().unwrap()),
            lease: None,
        };
        let value = connection_string(&endpoint, &route, 17).unwrap();
        assert!(value.contains("host='db.example'"));
        assert!(value.contains("hostaddr='127.0.0.1'"));
        assert!(value.contains("port='4567'"));
        assert!(value.contains("sslmode='require'"));
        assert!(value.contains("dbname='a\\' host=attacker \\\\'"));
        assert!(!value.contains("password="));
    }

    #[test]
    fn postgres_password_uses_a_private_passfile() {
        let profile =
            import_connection_url("postgresql://user@localhost/test?sslmode=disable", None)
                .unwrap()
                .profile;
        let endpoint = ResolvedEndpoint {
            profile,
            database: "test".into(),
            password: Some(secrecy::SecretString::from("hidden:password")),
        };
        let tool = NativeToolVersion {
            name: "pg_dump".into(),
            path: "/not/executed".into(),
            version: "pg_dump (PostgreSQL) 17.2".into(),
        };
        let process = client(&tool, &endpoint, &ConnectionRoute::direct()).unwrap();
        let command = process.command.as_std();
        assert!(
            command
                .get_args()
                .all(|value| !value.to_string_lossy().contains("hidden"))
        );
        let passfile = command
            .get_envs()
            .find(|(name, _)| *name == "PGPASSFILE")
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(passfile).unwrap(),
            "*:*:*:*:hidden\\:password\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(passfile).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
