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

    /// Load a theme-file-v1 JSON theme and watch it for changes.
    #[arg(long, global = true)]
    pub theme_file: Option<PathBuf>,

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
    /// Run a Language Server Protocol server over stdio.
    Lsp(LspArgs),
    /// Check or update the local LazyDB installation.
    Update(UpdateArgs),
    /// Remove the native LazyDB installation without deleting user data by default.
    Uninstall(UninstallArgs),
    /// Move the complete LazyDB application root to another directory.
    MigrateHome(MigrateHomeArgs),
}

#[derive(Debug, Args)]
pub struct LspArgs {
    /// Run the language server over stdin/stdout.
    #[arg(long)]
    pub stdio: bool,
    /// Project root used for workspace-relative services.
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// Explicit SQL dialect. The language server defaults to Generic.
    #[arg(long, value_enum)]
    pub dialect: Option<LspDialect>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum LspDialect {
    #[value(alias = "postgresql")]
    Postgres,
    #[value(name = "mysql", alias = "my-sql")]
    MySql,
    #[value(name = "sqlserver", alias = "sql-server")]
    SqlServer,
    Sqlite,
    #[default]
    Generic,
}

impl From<crate::profile::DatabaseKind> for LspDialect {
    fn from(kind: crate::profile::DatabaseKind) -> Self {
        match crate::sql::SqlDialect::for_database_kind(kind) {
            crate::sql::SqlDialect::Postgres => Self::Postgres,
            crate::sql::SqlDialect::MySql => Self::MySql,
            crate::sql::SqlDialect::SqlServer => Self::SqlServer,
            crate::sql::SqlDialect::Sqlite
            | crate::sql::SqlDialect::Generic
            | crate::sql::SqlDialect::Oracle => Self::Sqlite,
        }
    }
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

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Show the planned actions without changing the filesystem.
    #[arg(long)]
    pub dry_run: bool,
    /// Skip the interactive confirmation prompt.
    #[arg(long)]
    pub yes: bool,
    /// Also remove LazyDB user data and credentials when safely possible.
    #[arg(long)]
    pub purge: bool,
    /// Emit a machine-readable report.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct MigrateHomeArgs {
    /// Destination directory for the complete LazyDB application root.
    #[arg(long)]
    pub to: PathBuf,
    /// Show the migration plan without changing the filesystem.
    #[arg(long)]
    pub dry_run: bool,
    /// Confirm the filesystem migration.
    #[arg(long)]
    pub yes: bool,
    /// Emit a machine-readable report.
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
    /// Register an MCP server in an existing or new coding-agent configuration.
    Setup {
        #[arg(long = "client", value_enum)]
        client: Vec<McpClient>,
        /// Installation scope (interactive selection; project in scripts).
        #[arg(long, value_enum)]
        scope: Option<McpScope>,
        /// Explicit client configuration file (requires a single client).
        #[arg(long)]
        client_config: Option<PathBuf>,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, value_enum, default_value = "auto")]
        opencode_format: String,
        #[arg(long)]
        server_bin: Option<PathBuf>,
    },
    /// Inspect user and project MCP configuration without database I/O.
    Doctor {
        #[arg(long = "client", value_enum)]
        client: Vec<McpClient>,
        #[arg(long)]
        client_config: Option<PathBuf>,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        probe: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        opencode_bin: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum McpClient {
    ClaudeCode,
    Codex,
    Opencode,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpScope {
    #[default]
    Project,
    User,
    Local,
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
    pub revision: Option<&'a str>,
    pub drivers: &'a [&'a str],
}

#[derive(Debug, Serialize)]
pub struct Capabilities<'a> {
    pub version: &'a str,
    pub cli_api: u16,
    pub features: [&'a str; 7],
    pub drivers: [&'a str; 6],
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
        revision: option_env!("LAZYDB_GIT_REVISION"),
        drivers: &crate::db::descriptor::DRIVER_NAMES,
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
            "theme-file-v1",
            "lsp-v1",
        ],
        drivers: crate::db::descriptor::DRIVER_NAMES,
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
                detail: crate::db::descriptor::DRIVER_LIST,
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
            "lazydb {} (cli api {})\ndrivers: {}\nfeatures: mouse, read-only, context-help, profile-manager, system-keyring, theme-file-v1, lsp-v1",
            env!("CARGO_PKG_VERSION"),
            CLI_API_VERSION,
            crate::db::descriptor::DRIVER_LIST
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
                "LazyDB doctor: {status}\nlocale: {}\ndrivers: {}\ncredential_store: {} {} ({})",
                report.checks[0].status,
                crate::db::descriptor::DRIVER_LIST,
                report.credential_store.provider,
                report.credential_store.status,
                report.credential_store.detail,
            ))
        }
        Command::Agent { .. } | Command::Mcp { .. } => {
            Ok("This command requires asynchronous execution".to_owned())
        }
        Command::Lsp(_) => Ok("This command requires asynchronous execution".to_owned()),
        Command::Update(_) | Command::Uninstall(_) | Command::MigrateHome(_) => {
            Ok("This command requires asynchronous execution".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{
        CLI_API_VERSION, Cli, Command, MigrateHomeArgs, MotionMode, UninstallArgs, UpdateArgs,
        capabilities, render_command,
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
    fn parses_uninstall_options() {
        let cli = Cli::try_parse_from([
            "lazydb",
            "uninstall",
            "--dry-run",
            "--yes",
            "--purge",
            "--json",
        ])
        .unwrap();

        let Command::Uninstall(UninstallArgs {
            dry_run,
            yes,
            purge,
            json,
        }) = cli.command.unwrap()
        else {
            panic!("expected uninstall command");
        };
        assert!(dry_run && yes && purge && json);
    }

    #[test]
    fn parses_migrate_home_options() {
        let cli = Cli::try_parse_from([
            "lazydb",
            "migrate-home",
            "--to",
            "/tmp/lazydb",
            "--dry-run",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::MigrateHome(MigrateHomeArgs {
                dry_run: true,
                json: true,
                ..
            }))
        ));
    }

    #[test]
    fn parses_theme_file() {
        let cli = Cli::try_parse_from(["lazydb", "--theme-file", "/tmp/theme.json"]).unwrap();

        assert_eq!(
            cli.theme_file,
            Some(std::path::PathBuf::from("/tmp/theme.json"))
        );
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
            serde_json::json!([
                "postgres",
                "mysql",
                "mariadb",
                "oracle",
                "sqlserver",
                "sqlite"
            ])
        );
        assert_eq!(
            value["features"],
            serde_json::json!([
                "mouse",
                "read-only",
                "context-help",
                "profile-manager",
                "system-keyring",
                "theme-file-v1",
                "lsp-v1"
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
