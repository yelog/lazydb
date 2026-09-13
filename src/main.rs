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
                    client_config,
                    project,
                    dry_run,
                    yes,
                    json,
                    opencode_format,
                    opencode_bin,
                    server_bin,
                } => {
                    let output = lazydb::agent::setup::run_with_options(
                        lazydb::agent::setup::SetupOptions {
                            clients: client,
                            scope,
                            client_config,
                            project,
                            config: cli.config,
                            dry_run,
                            yes,
                            json,
                            opencode_format: if opencode_format == "auto" {
                                None
                            } else {
                                Some(match opencode_format.as_str() {
                                    "v1" => lazydb::agent::OpenCodeFormat::V1,
                                    "v2" => lazydb::agent::OpenCodeFormat::V2,
                                    _ => anyhow::bail!("invalid OpenCode format"),
                                })
                            },
                            opencode_bin,
                            server_bin,
                        },
                    )?;
                    println!("{output}");
                }
                lazydb::cli::McpCommand::Doctor {
                    client,
                    client_config,
                    project,
                    probe,
                    json,
                    opencode_bin,
                } => {
                    let output = lazydb::agent::doctor::run_with_options(
                        client,
                        project,
                        client_config,
                        probe,
                        json,
                        opencode_bin,
                    )
                    .await?;
                    println!("{output}");
                }
            },
            Command::Lsp(args) => {
                if !args.stdio {
                    anyhow::bail!("the LSP server currently requires --stdio");
                }
                lazydb::lsp::run(args, cli.config, cli.profile).await?;
            }
            Command::Update(args) => println!("{}", lazydb::update::run(args, cli.config).await?),
            Command::Uninstall(args) => {
                println!("{}", lazydb::uninstall::run(args, cli.config).await?);
            }
            Command::MigrateHome(args) => println!("{}", lazydb::migration::run(args)?),
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
