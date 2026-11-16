use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    agent::{context::AgentProjectContext, policy::WritePolicy, selection::select_profile},
    connections::DatabaseConnector,
    persistence::{
        credentials::CredentialResolver, local_credentials::LocalCredentialStore, paths::AppPaths,
        profiles::ProfileStore, secrets::NativeSecretStore, settings::AppSettings,
    },
    profile::ConnectionProfile,
    security::sanitize_terminal_text,
};

use super::{
    DataEndpoint, DataJobPlan, DataJobRecord, DataJobService, DataJobSpec, DataJobStatus,
    DataJobStore, DataOperation, NativeToolPaths,
};

#[derive(Debug, Args)]
#[command(about = "Plan and run native backups, restores, and same-engine copies")]
pub struct DataArgs {
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub project: Option<PathBuf>,
    #[command(subcommand)]
    pub command: DataCommand,
}

#[derive(Debug, Subcommand)]
pub enum DataCommand {
    #[command(about = "Back up a complete database; preview unless --execute is supplied")]
    Backup {
        #[arg(long)]
        connection: Option<String>,
        #[arg(long)]
        database: Option<String>,
        #[arg(long)]
        output: PathBuf,
        #[command(flatten)]
        options: RunOptions,
    },
    #[command(about = "Restore an archive to a new database or SQLite file")]
    Restore {
        #[arg(long)]
        connection: Option<String>,
        #[arg(long)]
        database: String,
        #[arg(long)]
        input: PathBuf,
        #[command(flatten)]
        options: RunOptions,
    },
    #[command(about = "Copy a complete database through a retained native archive")]
    Copy {
        #[arg(long)]
        source: String,
        #[arg(long)]
        source_database: Option<String>,
        #[arg(long)]
        destination: String,
        #[arg(long)]
        destination_database: String,
        #[arg(long)]
        output: PathBuf,
        #[command(flatten)]
        options: RunOptions,
    },
    #[command(about = "Inspect or execute a previously saved immutable plan")]
    Run {
        plan: PathBuf,
        #[arg(long)]
        execute: bool,
        #[arg(long, value_enum, default_value = "non-production")]
        write_policy: WritePolicy,
    },
    #[command(about = "List data tasks visible to this project")]
    Jobs {
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=1000))]
        limit: u32,
    },
    #[command(about = "Inspect a data task and its verification results")]
    Show { id: Uuid },
    #[command(about = "Build a fresh plan from a previous task")]
    Rerun {
        id: Uuid,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        destination_database: Option<String>,
        #[command(flatten)]
        options: RunOptions,
    },
}

#[derive(Debug, Args)]
pub struct RunOptions {
    #[arg(
        long,
        help = "Execute after successful preflight; otherwise only print a plan"
    )]
    pub execute: bool,
    #[arg(long, help = "Save the immutable plan to a new JSON file")]
    pub save_plan: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "non-production")]
    pub write_policy: WritePolicy,
    #[arg(long, help = "Path to pg_dump, mysqldump, or mariadb-dump")]
    pub dump_tool: Option<PathBuf>,
    #[arg(long, help = "Path to pg_restore, mysql, or mariadb")]
    pub restore_tool: Option<PathBuf>,
}

pub struct DataCliOutput {
    pub text: String,
    pub exit_code: i32,
}

#[derive(Serialize)]
struct Envelope<T: Serialize> {
    schema_version: u16,
    ok: bool,
    command: &'static str,
    data: Option<T>,
    error: Option<ErrorBody>,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

pub async fn run(
    args: DataArgs,
    config: Option<PathBuf>,
    profile: Option<String>,
    read_only: bool,
    url: Option<String>,
) -> anyhow::Result<DataCliOutput> {
    if url.is_some() {
        bail!(
            "Data tasks use saved connections. Add the connection first with `lazydb connections add`."
        );
    }
    let project = AgentProjectContext::resolve(args.project.as_deref())?;
    let paths = AppPaths::discover()?;
    let profile_path = config.unwrap_or_else(|| paths.profiles_file());
    let store_directory = profile_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join("data-jobs");
    let profile_store =
        ProfileStore::new(profile_path).with_credential_key_path(paths.credential_key_file());
    let collection = profile_store
        .load()
        .context("Cannot load saved connections")?;
    let mut profiles = project
        .visible_profiles(&collection.profiles)
        .into_iter()
        .map(|visible| visible.profile.clone())
        .collect::<Vec<_>>();
    if read_only {
        for profile in &mut profiles {
            profile.read_only = true;
        }
    }
    let credentials = CredentialResolver::new(
        Arc::new(NativeSecretStore),
        LocalCredentialStore::new(profile_store.credential_key_path(), "lazydb")
            .with_fallback_key_path(
                profile_store
                    .path()
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("credential.key"),
            ),
    );
    let ssh = AppSettings::load(paths.settings_file())
        .map(|settings| settings.connections.ssh)
        .unwrap_or_default();
    let connector = DatabaseConnector::with_ssh_config(ssh);
    let service = DataJobService::new(
        profiles,
        Arc::new(credentials),
        connector.clone(),
        DataJobStore::new(store_directory),
    );
    let cancellation = CancellationToken::new();
    let signal_cancellation = cancellation.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancellation.cancel();
        }
    });
    let result = dispatch(
        args.command,
        args.json,
        profile.as_deref(),
        &project,
        &service,
        cancellation,
    )
    .await;
    signal_task.abort();
    connector.shutdown().await;
    result
}

async fn dispatch(
    command: DataCommand,
    json: bool,
    profile_selector: Option<&str>,
    project: &AgentProjectContext,
    service: &DataJobService,
    cancellation: CancellationToken,
) -> anyhow::Result<DataCliOutput> {
    match command {
        DataCommand::Backup {
            connection,
            database,
            output,
            options,
        } => {
            let profile =
                selected_profile(service, project, connection.as_deref().or(profile_selector))?;
            let mut source = DataEndpoint::from_profile(profile);
            if let Some(database) = database {
                source.database = database;
            }
            let spec = DataJobSpec {
                operation: DataOperation::Backup,
                source: Some(source),
                destination: None,
                artifact: output,
                tools: tool_paths(&options),
            };
            plan_or_run("data.backup", spec, options, json, service, cancellation).await
        }
        DataCommand::Restore {
            connection,
            database,
            input,
            options,
        } => {
            let profile =
                selected_profile(service, project, connection.as_deref().or(profile_selector))?;
            let spec = DataJobSpec {
                operation: DataOperation::Restore,
                source: None,
                destination: Some(DataEndpoint {
                    profile_id: profile.id,
                    database,
                }),
                artifact: input,
                tools: tool_paths(&options),
            };
            plan_or_run("data.restore", spec, options, json, service, cancellation).await
        }
        DataCommand::Copy {
            source,
            source_database,
            destination,
            destination_database,
            output,
            options,
        } => {
            let source_profile = selected_profile(service, project, Some(&source))?;
            let destination_profile = selected_profile(service, project, Some(&destination))?;
            let mut source = DataEndpoint::from_profile(source_profile);
            if let Some(database) = source_database {
                source.database = database;
            }
            let spec = DataJobSpec {
                operation: DataOperation::Copy,
                source: Some(source),
                destination: Some(DataEndpoint {
                    profile_id: destination_profile.id,
                    database: destination_database,
                }),
                artifact: output,
                tools: tool_paths(&options),
            };
            plan_or_run("data.copy", spec, options, json, service, cancellation).await
        }
        DataCommand::Run {
            plan,
            execute,
            write_policy,
        } => {
            let bytes = super::store::read_metadata(&plan, 4 * 1024 * 1024, "Plan file")?;
            let plan: DataJobPlan =
                serde_json::from_slice(&bytes).context("The plan file is invalid")?;
            if !service.can_view(&plan.spec) {
                bail!("The plan is not available in this project.");
            }
            if execute {
                execute_plan("data.run", plan, write_policy, json, service, cancellation).await
            } else {
                render_plan("data.run", &plan, json, service)
            }
        }
        DataCommand::Jobs { limit } => {
            let jobs = service
                .store()
                .list()?
                .into_iter()
                .filter(|job| service.can_view(&job.plan.spec))
                .take(limit as usize)
                .collect::<Vec<_>>();
            let text = if json {
                envelope("data.jobs", &jobs)?
            } else if jobs.is_empty() {
                "No data tasks are available for this project.".into()
            } else {
                jobs.iter()
                    .map(|job| {
                        format!(
                            "{}  {:<9}  {:?}  {}",
                            job.id,
                            job.plan.spec.operation.label(),
                            job.status,
                            job.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Ok(DataCliOutput {
                text: sanitize_if_human(text, json),
                exit_code: 0,
            })
        }
        DataCommand::Show { id } => {
            let job = visible_job(service, id)?;
            Ok(DataCliOutput {
                text: if json {
                    envelope("data.show", &job)?
                } else {
                    render_job(&job)
                },
                exit_code: 0,
            })
        }
        DataCommand::Rerun {
            id,
            output,
            destination_database,
            options,
        } => {
            let job = visible_job(service, id)?;
            let mut spec = job.plan.spec;
            if let Some(output) = output {
                if spec.operation == DataOperation::Restore {
                    bail!(
                        "A restore reuses its input archive; --output only applies to backup and copy."
                    );
                }
                spec.artifact = output;
            }
            if let Some(database) = destination_database {
                let destination = spec
                    .destination
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("A backup has no destination database."))?;
                destination.database = database;
            }
            if options.dump_tool.is_some() {
                spec.tools.dump = options.dump_tool.clone();
            }
            if options.restore_tool.is_some() {
                spec.tools.restore = options.restore_tool.clone();
            }
            plan_or_run("data.rerun", spec, options, json, service, cancellation).await
        }
    }
}

async fn plan_or_run(
    command: &'static str,
    spec: DataJobSpec,
    options: RunOptions,
    json: bool,
    service: &DataJobService,
    cancellation: CancellationToken,
) -> anyhow::Result<DataCliOutput> {
    let plan = service
        .plan(spec, options.write_policy, cancellation.clone())
        .await?;
    if let Some(path) = options.save_plan {
        save_plan(&path, &plan)?;
    }
    if options.execute {
        execute_plan(
            command,
            plan,
            options.write_policy,
            json,
            service,
            cancellation,
        )
        .await
    } else {
        render_plan(command, &plan, json, service)
    }
}

async fn execute_plan(
    command: &'static str,
    plan: DataJobPlan,
    policy: WritePolicy,
    json: bool,
    service: &DataJobService,
    cancellation: CancellationToken,
) -> anyhow::Result<DataCliOutput> {
    let callback: Arc<dyn Fn(DataJobRecord) + Send + Sync> = Arc::new(move |record| {
        if !json {
            eprintln!("{}: {}", record.plan.spec.operation.label(), record.message);
        }
    });
    let record = service.run(plan, policy, cancellation, callback).await?;
    let ok = record.status.is_success();
    let exit_code = if ok {
        0
    } else if record.status == DataJobStatus::Cancelled {
        130
    } else {
        1
    };
    let text = if json {
        serde_json::to_string(&Envelope {
            schema_version: 1,
            ok,
            command,
            data: Some(&record),
            error: (!ok).then(|| ErrorBody {
                code: if record.status == DataJobStatus::Cancelled {
                    "cancelled"
                } else {
                    "task_failed"
                },
                message: record.message.clone(),
            }),
        })?
    } else {
        render_job(&record)
    };
    Ok(DataCliOutput { text, exit_code })
}

fn selected_profile<'service>(
    service: &'service DataJobService,
    project: &AgentProjectContext,
    selector: Option<&str>,
) -> anyhow::Result<&'service ConnectionProfile> {
    Ok(select_profile(&project.visible_profiles(service.profiles()), selector)?.profile)
}

fn tool_paths(options: &RunOptions) -> NativeToolPaths {
    NativeToolPaths {
        dump: options.dump_tool.clone(),
        restore: options.restore_tool.clone(),
    }
}

fn visible_job(service: &DataJobService, id: Uuid) -> anyhow::Result<DataJobRecord> {
    let job = service.store().load(id)?;
    if !service.can_view(&job.plan.spec) {
        bail!("The task is not available in this project.");
    }
    Ok(job)
}

fn save_plan(path: &Path, plan: &DataJobPlan) -> anyhow::Result<()> {
    let path = super::service::new_file_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Plan path has no parent."))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(plan)?)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(&path)
        .context("Plan files are never overwritten")?;
    super::store::sync_directory(parent)?;
    Ok(())
}

fn render_plan(
    command: &'static str,
    plan: &DataJobPlan,
    json: bool,
    service: &DataJobService,
) -> anyhow::Result<DataCliOutput> {
    if json {
        return Ok(DataCliOutput {
            text: envelope(command, plan)?,
            exit_code: 0,
        });
    }
    let mut lines = vec![format!("{} plan {}", plan.spec.operation.label(), plan.id)];
    for (label, endpoint) in [
        ("Source", &plan.spec.source),
        ("Destination", &plan.spec.destination),
    ] {
        if let Some(endpoint) = endpoint {
            let name = service
                .profiles()
                .iter()
                .find(|profile| profile.id == endpoint.profile_id)
                .map(|profile| profile.name.as_str())
                .unwrap_or("unavailable");
            lines.push(format!("{label}: {name} / {}", endpoint.database));
        }
    }
    lines.push(format!("Archive: {}", plan.spec.artifact.display()));
    lines.extend(plan.native.steps.iter().map(|step| format!("  {step}")));
    lines.extend(
        plan.native
            .warnings
            .iter()
            .map(|warning| format!("Warning: {warning}")),
    );
    lines.push(
        "Preview only. Add --execute to run, or --save-plan FILE to review and run the saved plan."
            .into(),
    );
    Ok(DataCliOutput {
        text: sanitize_terminal_text(&lines.join("\n")),
        exit_code: 0,
    })
}

fn render_job(job: &DataJobRecord) -> String {
    let mut lines = vec![
        format!(
            "Task {} · {} · {:?}",
            job.id,
            job.plan.spec.operation.label(),
            job.status
        ),
        job.message.clone(),
    ];
    if let Some(outcome) = &job.outcome {
        if let Some(artifact) = &outcome.artifact {
            lines.push(format!("Archive: {}", artifact.display()));
        }
        if let Some(bytes) = outcome.bytes {
            lines.push(format!("Bytes: {bytes}"));
        }
        if let Some(sha256) = &outcome.sha256 {
            lines.push(format!("SHA-256: {sha256}"));
        }
        lines.extend(
            outcome
                .verification
                .iter()
                .map(|verification| format!("Verified: {verification}")),
        );
        lines.extend(
            outcome
                .warnings
                .iter()
                .map(|warning| format!("Warning: {warning}")),
        );
    }
    if job.target_may_be_partial {
        lines.push(
            "The new target may contain partial data; it was not automatically deleted.".into(),
        );
    }
    sanitize_terminal_text(&lines.join("\n"))
}

fn envelope<T: Serialize>(command: &'static str, data: &T) -> anyhow::Result<String> {
    Ok(serde_json::to_string(&Envelope {
        schema_version: 1,
        ok: true,
        command,
        data: Some(data),
        error: None,
    })?)
}

fn sanitize_if_human(text: String, json: bool) -> String {
    if json {
        text
    } else {
        sanitize_terminal_text(&text)
    }
}

pub fn render_error(error: &anyhow::Error, json: bool) -> String {
    let message = sanitize_terminal_text(&format!("{error:#}"));
    if json {
        serde_json::to_string(&Envelope::<()> {
            schema_version: 1,
            ok: false,
            command: "data",
            data: None,
            error: Some(ErrorBody { code: "data_operation_failed", message }),
        }).unwrap_or_else(|_| r#"{"schema_version":1,"ok":false,"command":"data","error":{"code":"serialization_failed"}}"#.into())
    } else {
        message
    }
}
