use anyhow::Result;
use clap::Parser;
use lazydb::cli::{Cli, render_command};

#[tokio::main]
async fn main() -> Result<()> {
    lazydb::terminal::install_panic_hook();
    let cli = Cli::parse();

    if let Some(command) = &cli.command {
        println!("{}", render_command(command)?);
        return Ok(());
    }

    lazydb::runtime::run_tui(cli).await
}
