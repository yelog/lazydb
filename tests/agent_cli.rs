use clap::Parser;
use lazydb::cli::{AgentCommand, Cli, Command, ConnectionsCommand, McpCommand};

#[test]
fn parses_agent_query_inputs_and_defaults_to_read_only_server_policy() {
    let cli = Cli::try_parse_from(["lazydb", "agent", "query", "--sql", "SELECT 1"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Agent {
            command: AgentCommand::Query {
                sql: Some(_),
                file: None,
                ..
            }
        })
    ));

    let cli = Cli::try_parse_from(["lazydb", "mcp", "serve"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Mcp {
            command: McpCommand::Serve {
                write_policy: lazydb::agent::policy::WritePolicy::Deny,
                ..
            }
        })
    ));
}

#[test]
fn rejects_both_sql_and_file_inputs() {
    assert!(
        Cli::try_parse_from([
            "lazydb", "agent", "query", "--sql", "SELECT 1", "--file", "q.sql"
        ])
        .is_err()
    );
}

#[test]
fn parses_write_policy_and_file_input() {
    let cli = Cli::try_parse_from([
        "lazydb",
        "agent",
        "execute",
        "--file",
        "migration.sql",
        "--write-policy",
        "non-production",
        "--connection",
        "dev",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Agent {
            command: AgentCommand::Execute {
                write_policy: lazydb::agent::policy::WritePolicy::NonProduction,
                file: Some(_),
                ..
            }
        })
    ));
}

#[test]
fn parses_connection_add_with_global_url_and_json() {
    let cli = Cli::try_parse_from([
        "lazydb",
        "connections",
        "--json",
        "add",
        "--name",
        "app-dev",
        "--url",
        "postgresql://app@localhost:5432/app",
        "--scope",
        "project",
        "--project",
        ".",
        "--password-env",
        "APP_DB_PASSWORD",
    ])
    .unwrap();

    assert_eq!(
        cli.url.as_deref(),
        Some("postgresql://app@localhost:5432/app")
    );
    assert!(matches!(
        cli.command,
        Some(Command::Connections {
            json: true,
            command: ConnectionsCommand::Add { .. }
        })
    ));
}

#[test]
fn parses_connection_read_commands_and_upsert() {
    let cli = Cli::try_parse_from([
        "lazydb",
        "connections",
        "add",
        "--name",
        "app-dev",
        "--url",
        "sqlite::memory:",
        "--scope",
        "global",
        "--upsert",
        "--read-write",
        "--json",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Connections {
            json: true,
            command: ConnectionsCommand::Add { .. }
        })
    ));

    assert!(Cli::try_parse_from(["lazydb", "connections", "list", "--all"]).is_ok());
    assert!(Cli::try_parse_from(["lazydb", "connections", "show", "app-dev"]).is_ok());
    assert!(
        Cli::try_parse_from(["lazydb", "connections", "test", "app-dev", "--timeout", "5"]).is_ok()
    );
}

#[test]
fn rejects_conflicting_connection_password_sources() {
    assert!(
        Cli::try_parse_from([
            "lazydb",
            "connections",
            "add",
            "--name",
            "demo",
            "--url",
            "sqlite::memory:",
            "--password-env",
            "PASSWORD",
            "--password-stdin"
        ])
        .is_err()
    );
}

#[test]
fn connections_global_flags_and_password_modes_parse() {
    let cli = Cli::try_parse_from([
        "lazydb",
        "--read-only",
        "connections",
        "add",
        "--name",
        "demo",
        "--url",
        "sqlite::memory:",
    ])
    .unwrap();
    assert!(cli.read_only);
    assert!(
        Cli::try_parse_from([
            "lazydb",
            "connections",
            "add",
            "--name",
            "demo",
            "--url",
            "sqlite::memory:",
            "--read-write",
        ])
        .is_ok()
    );
}
