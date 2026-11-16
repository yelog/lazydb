mod common;
mod mysql;
mod postgres;
mod sqlite;

use anyhow::{Context, Result, bail, ensure};
use tokio_util::sync::CancellationToken;

use crate::{
    connections::DatabaseConnector,
    data_ops::{
        DataJobSpec, DataJobStatus, DataOperation, NativeOutcome, NativePlan, ProgressSink,
        ResolvedEndpoint, supports_native_data_operations,
    },
    profile::DatabaseKind,
};

pub async fn plan(
    spec: &DataJobSpec,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<NativePlan> {
    let redactor = common::Redactor::new(&[source, destination]);
    preflight(spec, source, destination, connector, cancellation)
        .await
        .map_err(|error| redactor.error(error))
}

async fn preflight(
    spec: &DataJobSpec,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
) -> Result<NativePlan> {
    common::check_cancel(cancellation)?;
    let engine = validate_spec(spec, source, destination)?;
    if spec.operation == DataOperation::Restore {
        common::input_path(&spec.artifact)?;
    } else {
        common::output_path(&spec.artifact)?;
    }
    let mut plan = match engine {
        DatabaseKind::Sqlite => {
            Box::pin(sqlite::plan(spec, source, destination, cancellation)).await?
        }
        DatabaseKind::Postgres => {
            Box::pin(postgres::plan(
                spec,
                source,
                destination,
                connector,
                cancellation,
            ))
            .await?
        }
        DatabaseKind::MySql | DatabaseKind::MariaDb => {
            Box::pin(mysql::plan(
                spec,
                source,
                destination,
                connector,
                cancellation,
            ))
            .await?
        }
        _ => bail!("native data operations are not implemented for {engine:?}"),
    };
    if spec.operation == DataOperation::Restore {
        let (bytes, digest) = common::fingerprint(&spec.artifact, cancellation).await?;
        plan.steps.push(common::fingerprint_step(bytes, &digest));
    }
    common::check_cancel(cancellation)?;
    Ok(plan)
}

pub async fn execute(
    spec: &DataJobSpec,
    plan: &NativePlan,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
    connector: &DatabaseConnector,
    cancellation: &CancellationToken,
    progress: ProgressSink,
) -> Result<NativeOutcome> {
    let redactor = common::Redactor::new(&[source, destination]);
    let operation = async {
        common::progress(
            &progress,
            DataJobStatus::Preflight,
            "Revalidating native preflight",
            None,
        );
        let current = preflight(spec, source, destination, connector, cancellation).await?;
        ensure!(
            current.engine == plan.engine && current.tools == plan.tools,
            "native tools or engine changed after review; create a new plan"
        );
        ensure!(
            current.steps == plan.steps && current.warnings == plan.warnings,
            "native preflight changed after review; create a new plan"
        );
        let pinned = if spec.operation == DataOperation::Restore {
            let input = common::PinnedInput::new(&spec.artifact, cancellation).await?;
            ensure!(
                plan.steps
                    .contains(&common::fingerprint_step(input.bytes, &input.sha256)),
                "restore input changed after review"
            );
            Some(input)
        } else {
            None
        };
        match plan.engine {
            DatabaseKind::Sqlite => {
                Box::pin(sqlite::execute(
                    spec,
                    plan,
                    source,
                    destination,
                    pinned.as_ref(),
                    cancellation,
                    &progress,
                ))
                .await
            }
            DatabaseKind::Postgres => {
                Box::pin(postgres::execute(
                    spec,
                    plan,
                    (source, destination),
                    pinned.as_ref(),
                    connector,
                    cancellation,
                    &progress,
                ))
                .await
            }
            DatabaseKind::MySql | DatabaseKind::MariaDb => {
                Box::pin(mysql::execute(
                    spec,
                    plan,
                    (source, destination),
                    pinned.as_ref(),
                    connector,
                    cancellation,
                    &progress,
                ))
                .await
            }
            _ => bail!("unsupported native database engine"),
        }
    };
    operation.await.map_err(|error| redactor.error(error))
}

fn validate_spec(
    spec: &DataJobSpec,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
) -> Result<DatabaseKind> {
    match spec.operation {
        DataOperation::Backup => ensure!(
            source.is_some() && destination.is_none() && spec.destination.is_none(),
            "backup requires exactly one source"
        ),
        DataOperation::Restore => ensure!(
            source.is_none() && destination.is_some() && spec.source.is_none(),
            "restore requires exactly one new destination"
        ),
        DataOperation::Copy => ensure!(
            source.is_some() && destination.is_some(),
            "copy requires a source and a new destination"
        ),
    }
    for (requested, endpoint) in [
        (spec.source.as_ref(), source),
        (spec.destination.as_ref(), destination),
    ] {
        match (requested, endpoint) {
            (Some(requested), Some(endpoint)) => {
                ensure!(
                    requested.profile_id == endpoint.profile.id
                        && requested.database == endpoint.database,
                    "resolved endpoint does not match the reviewed request"
                );
                common::clean_text(&endpoint.database)?;
                if endpoint.profile.kind != DatabaseKind::Sqlite {
                    common::quote_identifier(
                        &endpoint.database,
                        endpoint.profile.kind != DatabaseKind::Postgres,
                    )?;
                    common::clean_text(
                        endpoint
                            .profile
                            .host
                            .as_deref()
                            .context("native client requires an explicit host")?,
                    )?;
                    common::clean_text(
                        endpoint
                            .profile
                            .user
                            .as_deref()
                            .context("native client requires an explicit user")?,
                    )?;
                }
            }
            (None, None) => {}
            _ => bail!("resolved endpoints do not match the requested operation"),
        }
    }
    let engine = source
        .or(destination)
        .context("operation has no endpoint")?
        .profile
        .kind;
    ensure!(
        supports_native_data_operations(engine),
        "native data operations do not support {engine:?}"
    );
    if let Some(destination) = destination {
        ensure!(
            !destination.profile.read_only,
            "destination profile is read-only"
        );
        ensure!(
            destination.profile.kind == engine,
            "copy requires exactly the same database engine"
        );
        if let Some(source) = source {
            ensure!(
                source.profile.kind == destination.profile.kind,
                "cross-engine copy is not supported"
            );
            if engine != DatabaseKind::Sqlite {
                let same_server = source.profile.id == destination.profile.id
                    || (source.profile.host.as_ref().map(|host| host.to_lowercase())
                        == destination
                            .profile
                            .host
                            .as_ref()
                            .map(|host| host.to_lowercase())
                        && source.profile.port.unwrap_or(default_port(engine))
                            == destination.profile.port.unwrap_or(default_port(engine))
                        && source.profile.ssh == destination.profile.ssh);
                ensure!(
                    !same_server || source.database != destination.database,
                    "source and destination must be different databases"
                );
            }
        }
    }
    Ok(engine)
}

fn default_port(engine: DatabaseKind) -> u16 {
    if engine == DatabaseKind::Postgres {
        5432
    } else {
        3306
    }
}
