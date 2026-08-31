use clap::Parser;
use lazydb::cli::{AgentCommand, Cli, Command, McpCommand};

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
