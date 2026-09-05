use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::persistence::secrets::secret_store_diagnostic;
use crate::ui::icons::IconMode;

pub const CLI_API_VERSION: u16 = 1;

#[derive(Debug, Parser)]
#[command(
    name = "lazydb",
    version,
    about = "A keyboard-first terminal database IDE"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Open an ad-hoc connection without persisting it.
    #[arg(long, global = true, hide = true)]
    pub url: Option<String>,

    #[arg(long, global = true)]
    pub read_only: bool,

    #[arg(long, global = true, value_enum)]
    pub mouse: Option<MouseMode>,

    #[arg(long, global = true, value_enum)]
    pub color: Option<ColorMode>,

    #[arg(long, global = true, value_enum)]
    pub icons: Option<IconMode>,

    #[arg(long, global = true, value_enum)]
    pub motion: Option<MotionMode>,

    #[arg(long, global = true, value_enum)]
    pub confirm_execution: Option<ConfirmationPolicy>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ConfirmationPolicy {
    #[default]
    #[value(name = "risky")]
    #[serde(rename = "risky")]
    RiskyOnly,
    #[value(name = "always")]
    Always,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum MouseMode {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum MotionMode {
    #[default]
    Full,
    Reduced,
    Off,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print version information.
    Version {
        #[arg(long)]
        json: bool,
    },
    /// Print the stable CLI capabilities contract.
    Capabilities {
        #[arg(long)]
        json: bool,
    },
    /// Check the local LazyDB environment without connecting by default.
    Doctor {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Run a machine-readable database operation for the current project.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Run an agent protocol server.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Check or update the local LazyDB installation.
    Update(UpdateArgs),
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Check for an update without applying one.
    #[arg(long)]
    pub check: bool,
    /// Select the release channel.
    #[arg(long, value_enum)]
    pub channel: Option<crate::update::UpdateChannel>,
    /// Permit a future updater to select an older version.
    #[arg(long, conflicts_with = "check")]
    pub allow_downgrade: bool,
    /// Emit the stable machine-readable report contract.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    Context(AgentTargetArgs),
    Connections(ProjectArgs),
    SchemaSearch {
        query: String,
        #[command(flatten)]
        target: AgentTargetArgs,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Query {
        #[command(flatten)]
        target: AgentTargetArgs,
        #[arg(long, conflicts_with = "file", required_unless_present = "file")]
        sql: Option<String>,
        #[arg(long, conflicts_with = "sql", required_unless_present = "sql")]
        file: Option<PathBuf>,
    },
    Execute {
        #[command(flatten)]
        target: AgentTargetArgs,
        #[arg(long, conflicts_with = "file", required_unless_present = "file")]
        sql: Option<String>,
        #[arg(long, conflicts_with = "sql", required_unless_present = "sql")]
        file: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = crate::agent::policy::WritePolicy::Deny)]
        write_policy: crate::agent::policy::WritePolicy,
    },
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    Serve {
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        connection: Option<String>,
        #[arg(long, value_enum, default_value_t = crate::agent::policy::WritePolicy::Deny)]
        write_policy: crate::agent::policy::WritePolicy,
    },
    /// Configure a project-scoped MCP server for coding agents.
    Setup {
        #[arg(long = "client", value_enum)]
        client: Vec<McpClient>,
        #[arg(long, default_value = "project", value_enum)]
        scope: McpScope,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
    /// Inspect project-scoped MCP configuration without database I/O.
    Doctor {
        #[arg(long = "client", value_enum)]
        client: Vec<McpClient>,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        probe: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum McpClient {
    ClaudeCode,
    Codex,
    Opencode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum McpScope {
    #[default]
    Project,
}

#[derive(Clone, Debug, Args)]
pub struct ProjectArgs {
    #[arg(long)]
    pub project: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub struct AgentTargetArgs {
    #[arg(long)]
    pub project: Option<PathBuf>,
    #[arg(long)]
    pub connection: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VersionInfo<'a> {
    pub version: &'a str,
    pub cli_api: u16,
}

#[derive(Debug, Serialize)]
pub struct Capabilities<'a> {
    pub version: &'a str,
    pub cli_api: u16,
    pub features: [&'a str; 5],
    pub drivers: [&'a str; 3],
}

#[derive(Debug, Serialize)]
pub struct DoctorReport<'a> {
    pub ok: bool,
    pub version: &'a str,
    pub cli_api: u16,
    pub profile: Option<&'a str>,
    pub checks: [DoctorCheck<'a>; 3],
    pub credential_store: CredentialStoreReport,
}

#[derive(Debug, Serialize)]
pub struct CredentialStoreReport {
    pub provider: &'static str,
    pub status: &'static str,
    pub detail: &'static str,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck<'a> {
    pub name: &'a str,
    pub status: &'a str,
    pub detail: &'a str,
}

pub fn version_info() -> VersionInfo<'static> {
    VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        cli_api: CLI_API_VERSION,
    }
}

pub fn capabilities() -> Capabilities<'static> {
    Capabilities {
        version: env!("CARGO_PKG_VERSION"),
        cli_api: CLI_API_VERSION,
        features: [
            "mouse",
            "read-only",
            "context-help",
            "profile-manager",
            "system-keyring",
        ],
        drivers: ["postgres", "mysql", "sqlite"],
    }
}

pub fn doctor_report(profile: Option<&str>) -> DoctorReport<'_> {
    let locale_is_utf8 = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_CTYPE"))
        .or_else(|_| std::env::var("LANG"))
        .map(|value| value.to_ascii_lowercase().contains("utf-8") || value.contains("utf8"))
        .unwrap_or(true);

    let secret_store = secret_store_diagnostic();
    DoctorReport {
        ok: locale_is_utf8,
        version: env!("CARGO_PKG_VERSION"),
        cli_api: CLI_API_VERSION,
        profile,
        credential_store: CredentialStoreReport {
            provider: secret_store.provider,
            status: secret_store.status,
            detail: secret_store.detail,
        },
        checks: [
            DoctorCheck {
                name: "locale",
                status: if locale_is_utf8 { "ok" } else { "warning" },
                detail: if locale_is_utf8 {
                    "UTF-8 locale detected"
                } else {
                    "UTF-8 locale not detected"
                },
            },
            DoctorCheck {
                name: "drivers",
                status: "ok",
                detail: "postgres,mysql,sqlite",
            },
            DoctorCheck {
                name: "credential_store",
                status: secret_store.status,
                detail: secret_store.detail,
            },
        ],
    }
}

pub fn render_command(command: &Command) -> Result<String, serde_json::Error> {
    match command {
        Command::Version { json: true } => serde_json::to_string(&version_info()),
        Command::Version { json: false } => Ok(format!("lazydb {}", env!("CARGO_PKG_VERSION"))),
        Command::Capabilities { json: true } => serde_json::to_string(&capabilities()),
        Command::Capabilities { json: false } => Ok(format!(
            "lazydb {} (cli api {})\ndrivers: postgres, mysql, sqlite\nfeatures: mouse, read-only, context-help, profile-manager, system-keyring",
            env!("CARGO_PKG_VERSION"),
            CLI_API_VERSION
        )),
        Command::Doctor {
            json: true,
            profile,
        } => serde_json::to_string(&doctor_report(profile.as_deref())),
        Command::Doctor {
            json: false,
            profile,
        } => {
            let report = doctor_report(profile.as_deref());
            let status = if report.ok { "ok" } else { "warning" };
            Ok(format!(
                "LazyDB doctor: {status}\nlocale: {}\ndrivers: postgres, mysql, sqlite\ncredential_store: {} {} ({})",
                report.checks[0].status,
                report.credential_store.provider,
                report.credential_store.status,
                report.credential_store.detail,
            ))
        }
        Command::Agent { .. } | Command::Mcp { .. } => {
            Ok("This command requires asynchronous execution".to_owned())
        }
        Command::Update(_) => Ok("This command requires asynchronous execution".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        CLI_API_VERSION, Cli, Command, MotionMode, UpdateArgs, capabilities, render_command,
    };
    use crate::ui::icons::IconMode;

    #[test]
    fn parses_direct_connection_url() {
        let cli =
            Cli::try_parse_from(["lazydb", "--url", "sqlite://demo.db", "--read-only"]).unwrap();

        assert_eq!(cli.url.as_deref(), Some("sqlite://demo.db"));
        assert!(cli.read_only);
    }

    #[test]
    fn parses_icon_modes() {
        assert_eq!(Cli::try_parse_from(["lazydb"]).unwrap().icons, None);
        assert_eq!(
            Cli::try_parse_from(["lazydb", "--icons", "unicode"])
                .unwrap()
                .icons,
            Some(IconMode::Unicode)
        );
        assert_eq!(
            Cli::try_parse_from(["lazydb", "--icons", "ascii"])
                .unwrap()
                .icons,
            Some(IconMode::Ascii)
        );
        assert_eq!(
            Cli::try_parse_from(["lazydb", "--icons", "nerd-font"])
                .unwrap()
                .icons,
            Some(IconMode::NerdFont)
        );
        assert!(Cli::try_parse_from(["lazydb", "--icons", "emoji"]).is_err());
    }

    #[test]
    fn omitted_motion_does_not_override_configuration() {
        assert_eq!(Cli::try_parse_from(["lazydb"]).unwrap().motion, None);
    }

    #[test]
    fn parses_all_motion_modes() {
        for (value, expected) in [
            ("full", MotionMode::Full),
            ("reduced", MotionMode::Reduced),
            ("off", MotionMode::Off),
        ] {
            assert_eq!(
                Cli::try_parse_from(["lazydb", "--motion", value])
                    .unwrap()
                    .motion,
                Some(expected)
            );
        }
        assert!(Cli::try_parse_from(["lazydb", "--motion", "none"]).is_err());
    }

    #[test]
    fn parses_machine_readable_capabilities() {
        let cli = Cli::try_parse_from(["lazydb", "capabilities", "--json"]).unwrap();

        assert!(matches!(
            cli.command,
            Some(Command::Capabilities { json: true })
        ));
    }

    #[test]
    fn capabilities_contract_is_stable() {
        let value = serde_json::to_value(capabilities()).unwrap();

        assert_eq!(value["cli_api"], CLI_API_VERSION);
        assert_eq!(
            value["drivers"],
            serde_json::json!(["postgres", "mysql", "sqlite"])
        );
        assert_eq!(
            value["features"],
            serde_json::json!([
                "mouse",
                "read-only",
                "context-help",
                "profile-manager",
                "system-keyring"
            ])
        );
    }

    #[test]
    fn renders_json_as_a_single_line() {
        let output = render_command(&Command::Version { json: true }).unwrap();

        assert!(!output.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output).unwrap()["cli_api"],
            CLI_API_VERSION
        );
    }

    #[test]
    fn parses_update_arguments_and_channels() {
        assert!(matches!(
            Cli::try_parse_from(["lazydb", "update"]).unwrap().command,
            Some(Command::Update(UpdateArgs {
                check: false,
                channel: None,
                allow_downgrade: false,
                json: false,
            }))
        ));
        assert!(matches!(
            Cli::try_parse_from(["lazydb", "update", "--check", "--json"])
                .unwrap()
                .command,
            Some(Command::Update(UpdateArgs {
                check: true,
                json: true,
                ..
            }))
        ));
        assert!(matches!(
            Cli::try_parse_from(["lazydb", "update", "--channel", "beta"])
                .unwrap()
                .command,
            Some(Command::Update(UpdateArgs {
                channel: Some(crate::update::UpdateChannel::Beta),
                ..
            }))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "lazydb",
                "update",
                "--channel",
                "stable",
                "--allow-downgrade",
                "--json"
            ])
            .unwrap()
            .command,
            Some(Command::Update(UpdateArgs {
                allow_downgrade: true,
                json: true,
                ..
            }))
        ));
        assert!(Cli::try_parse_from(["lazydb", "update", "--check", "--allow-downgrade"]).is_err());
        assert!(Cli::try_parse_from(["lazydb", "update", "--channel", "nightly"]).is_err());
    }
}
