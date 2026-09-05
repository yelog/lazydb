use anyhow::Result;
use clap::Parser;
use lazydb::cli::{Cli, Command, render_command};

#[tokio::main]
async fn main() -> Result<()> {
    lazydb::terminal::install_panic_hook();
    let cli = Cli::parse();

    if let Some(command) = cli.command {
        match command {
            Command::Agent { command } => {
                println!("{}", lazydb::agent::cli::run(command, cli.config).await?);
            }
            Command::Mcp { command } => match command {
                lazydb::cli::McpCommand::Serve {
                    project,
                    connection,
                    write_policy,
                } => {
                    lazydb::agent::mcp::run(project, connection, write_policy, cli.config).await?;
                }
                lazydb::cli::McpCommand::Setup {
                    client,
                    scope,
                    project,
                    dry_run,
                    yes,
                    json,
                } => {
                    let output = lazydb::agent::setup::run(
                        client, scope, project, cli.config, dry_run, yes, json,
                    )?;
                    println!("{output}");
                }
                lazydb::cli::McpCommand::Doctor {
                    client,
                    project,
                    probe,
                    json,
                } => {
                    let output =
                        lazydb::agent::doctor::run(client, project, cli.config, probe, json)
                            .await?;
                    println!("{output}");
                }
            },
            Command::Update(args) => println!("{}", lazydb::update::run(args, cli.config).await?),
            command => println!("{}", render_command(&command)?),
        }
        return Ok(());
    }

    match lazydb::runtime::run_tui(cli).await? {
        lazydb::runtime::RunOutcome::Exit => Ok(()),
        lazydb::runtime::RunOutcome::Restart { executable } => {
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;

                let error = std::process::Command::new(executable)
                    .args(std::env::args_os().skip(1))
                    .exec();
                Err(error.into())
            }
            #[cfg(not(unix))]
            {
                let status = std::process::Command::new(executable)
                    .args(std::env::args_os().skip(1))
                    .status()?;
                if status.success() {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("restart process exited with {status}"))
                }
            }
        }
    }
}
