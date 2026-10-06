use anyhow::Result;
use clap::Parser;
use lazydb::cli::{Cli, Command, render_command};

#[tokio::main]
async fn main() -> Result<()> {
    lazydb::terminal::install_panic_hook();
    let cli = parse_cli();
    let Cli {
        config,
        profile,
        url,
        read_only,
        mouse,
        color,
        theme_file,
        icons,
        motion,
        confirm_execution,
        command,
    } = cli;

    if let Some(command) = command {
        match command {
            Command::Agent { command } => {
                println!("{}", lazydb::agent::cli::run(command, config).await?);
            }
            Command::Connections { command, json } => {
                let command_name = command.name();
                match lazydb::connections::cli::run(config, url, profile, read_only, command, json)
                    .await
                {
                    Ok(output) => println!("{output}"),
                    Err(error) => {
                        let output =
                            lazydb::connections::cli::render_error(command_name, &error, json);
                        if json {
                            println!("{output}");
                        } else {
                            eprintln!("{output}");
                        }
                        std::process::exit(error.exit_code);
                    }
                }
            }
            Command::Mcp { command } => match command {
                lazydb::cli::McpCommand::Serve {
                    project,
                    connection,
                    write_policy,
                } => {
                    lazydb::agent::mcp::run(project, connection, write_policy, config).await?;
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
                            config,
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
                lazydb::lsp::run(args, config, profile).await?;
            }
            Command::Update(args) => println!("{}", lazydb::update::run(args, config).await?),
            Command::Uninstall(args) => {
                println!("{}", lazydb::uninstall::run(args, config).await?);
            }
            Command::MigrateHome(args) => println!("{}", lazydb::migration::run(args)?),
            command => println!("{}", render_command(&command)?),
        }
        return Ok(());
    }

    let cli = Cli {
        config,
        profile,
        url,
        read_only,
        command: None,
        mouse,
        color,
        theme_file,
        icons,
        motion,
        confirm_execution,
    };
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

fn parse_cli() -> Cli {
    let args = std::env::args_os().collect::<Vec<_>>();
    match Cli::try_parse_from(args.clone()) {
        Ok(cli) => cli,
        Err(parse_error) => {
            use clap::error::ErrorKind;
            if matches!(
                parse_error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                parse_error.exit();
            }
            if let Some(command) = connections_parse_command(&args) {
                let json = args.iter().any(|arg| arg == "--json");
                let error = lazydb::connections::cli::ConnectionCliError {
                    code: "invalid_arguments",
                    message: "Invalid connections command arguments.".to_owned(),
                    exit_code: 2,
                };
                let output = lazydb::connections::cli::render_error(command, &error, json);
                if json {
                    println!("{output}");
                } else {
                    eprintln!("{output}");
                }
                std::process::exit(2);
            }
            parse_error.exit()
        }
    }
}

fn connections_parse_command(args: &[std::ffi::OsString]) -> Option<&'static str> {
    let mut index = 1;
    while let Some(argument) = args.get(index).and_then(|argument| argument.to_str()) {
        if matches!(
            argument,
            "--config"
                | "--profile"
                | "--url"
                | "--mouse"
                | "--color"
                | "--theme-file"
                | "--icons"
                | "--motion"
                | "--confirm-execution"
        ) {
            index = index.saturating_add(2);
            continue;
        }
        if argument.starts_with('-') {
            index += 1;
            continue;
        }
        if argument != "connections" {
            return None;
        }
        let mut subcommand_index = index + 1;
        while args
            .get(subcommand_index)
            .and_then(|argument| argument.to_str())
            .is_some_and(|argument| argument.starts_with('-'))
        {
            subcommand_index += 1;
        }
        let Some(subcommand) = args.get(subcommand_index).and_then(|value| value.to_str()) else {
            return Some("connections");
        };
        return match subcommand {
            "add" => Some("connections.add"),
            "list" => Some("connections.list"),
            "show" => Some("connections.show"),
            "test" => Some("connections.test"),
            _ => Some("connections"),
        };
    }
    None
}
