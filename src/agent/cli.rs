use std::fs;
use std::path::Path;

use crate::agent::service::AgentService;
use crate::cli::{AgentCommand, AgentTargetArgs, ProjectArgs};

pub async fn run(
    command: AgentCommand,
    config: Option<std::path::PathBuf>,
) -> anyhow::Result<String> {
    match command {
        AgentCommand::Connections(ProjectArgs { project }) => {
            let service = AgentService::load(project.as_deref(), config)?;
            let result = Ok(serde_json::to_string(&service.connections())?);
            finish(service, result).await
        }
        AgentCommand::Context(AgentTargetArgs {
            project,
            connection,
        }) => {
            let service = AgentService::load(project.as_deref(), config)?;
            let result = Ok(serde_json::to_string(
                &service.context(connection.as_deref())?,
            )?);
            finish(service, result).await
        }
        AgentCommand::SchemaSearch {
            query,
            target,
            limit,
        } => {
            let service = AgentService::load(target.project.as_deref(), config)?;
            let result = async {
                Ok(serde_json::to_string(
                    &service
                        .search_schema(target.connection.as_deref(), query, limit)
                        .await?,
                )?)
            }
            .await;
            finish(service, result).await
        }
        AgentCommand::MongoQuery {
            target,
            database,
            command,
            file,
        } => {
            let service = AgentService::load(target.project.as_deref(), config)?;
            let result = async {
                let input = read_sql(
                    file.as_deref(),
                    command.as_deref(),
                    target.project.as_deref(),
                )?;
                Ok(serde_json::to_string(
                    &service
                        .mongo_query(target.connection.as_deref(), &database, &input)
                        .await?,
                )?)
            }
            .await;
            finish(service, result).await
        }
        AgentCommand::MongoExecute {
            target,
            database,
            command,
            file,
            write_policy,
        }
        | AgentCommand::Mongo {
            command:
                crate::cli::MongoAgentCommand::Execute {
                    target,
                    database,
                    command,
                    file,
                    write_policy,
                },
        } => {
            let service = AgentService::load(target.project.as_deref(), config)?;
            let result = async {
                let input = read_sql(
                    file.as_deref(),
                    command.as_deref(),
                    target.project.as_deref(),
                )?;
                Ok(serde_json::to_string(
                    &service
                        .mongo_execute_change(
                            target.connection.as_deref(),
                            &database,
                            &input,
                            write_policy,
                        )
                        .await?,
                )?)
            }
            .await;
            finish(service, result).await
        }
        AgentCommand::Query { target, sql, file } => {
            let service = AgentService::load(target.project.as_deref(), config)?;
            let result = async {
                let sql = read_sql(file.as_deref(), sql.as_deref(), target.project.as_deref())?;
                Ok(serde_json::to_string(
                    &service.query(target.connection.as_deref(), &sql).await?,
                )?)
            }
            .await;
            finish(service, result).await
        }
        AgentCommand::Execute {
            target,
            sql,
            file,
            write_policy,
        } => {
            let service = AgentService::load(target.project.as_deref(), config)?;
            let result = async {
                let sql = read_sql(file.as_deref(), sql.as_deref(), target.project.as_deref())?;
                Ok(serde_json::to_string(
                    &service
                        .execute(target.connection.as_deref(), &sql, write_policy)
                        .await?,
                )?)
            }
            .await;
            finish(service, result).await
        }
    }
}

async fn finish(service: AgentService, result: anyhow::Result<String>) -> anyhow::Result<String> {
    service.shutdown().await;
    result
}

fn read_sql(
    file: Option<&Path>,
    sql: Option<&str>,
    project: Option<&Path>,
) -> anyhow::Result<String> {
    if let Some(sql) = sql {
        return Ok(sql.to_owned());
    }
    let file = file.ok_or_else(|| anyhow::anyhow!("SQL input is required"))?;
    let project = project
        .ok_or_else(|| anyhow::anyhow!("--project is required with --file"))?
        .canonicalize()?;
    let file = file.canonicalize()?;
    if !file.starts_with(&project) {
        anyhow::bail!("SQL file is outside the project root");
    }
    Ok(fs::read_to_string(file)?)
}
