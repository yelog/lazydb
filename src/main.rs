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
            },
            Command::Update(args) => println!("{}", lazydb::update::run(args, cli.config).await?),
            command => println!("{}", render_command(&command)?),
        }
        return Ok(());
    }

    lazydb::runtime::run_tui(cli).await
}
