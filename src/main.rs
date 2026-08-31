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
            Command::Mcp { .. } => {
                println!("MCP server is not implemented yet");
            }
            command => println!("{}", render_command(&command)?),
        }
        return Ok(());
    }

    lazydb::runtime::run_tui(cli).await
}
