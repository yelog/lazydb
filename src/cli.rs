use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;

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

    #[arg(long, global = true, value_enum, default_value_t = MouseMode::Auto)]
    pub mouse: MouseMode,

    #[arg(long, global = true, value_enum, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum MouseMode {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
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
    pub features: [&'a str; 3],
    pub drivers: [&'a str; 3],
}

#[derive(Debug, Serialize)]
pub struct DoctorReport<'a> {
    pub ok: bool,
    pub version: &'a str,
    pub cli_api: u16,
    pub profile: Option<&'a str>,
    pub checks: [DoctorCheck<'a>; 2],
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
        features: ["mouse", "read-only", "context-help"],
        drivers: ["postgres", "mysql", "sqlite"],
    }
}

pub fn doctor_report(profile: Option<&str>) -> DoctorReport<'_> {
    let locale_is_utf8 = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_CTYPE"))
        .or_else(|_| std::env::var("LANG"))
        .map(|value| value.to_ascii_lowercase().contains("utf-8") || value.contains("utf8"))
        .unwrap_or(true);

    DoctorReport {
        ok: locale_is_utf8,
        version: env!("CARGO_PKG_VERSION"),
        cli_api: CLI_API_VERSION,
        profile,
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
        ],
    }
}

pub fn render_command(command: &Command) -> Result<String, serde_json::Error> {
    match command {
        Command::Version { json: true } => serde_json::to_string(&version_info()),
        Command::Version { json: false } => Ok(format!("lazydb {}", env!("CARGO_PKG_VERSION"))),
        Command::Capabilities { json: true } => serde_json::to_string(&capabilities()),
        Command::Capabilities { json: false } => Ok(format!(
            "lazydb {} (cli api {})\ndrivers: postgres, mysql, sqlite\nfeatures: mouse, read-only, context-help",
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
                "LazyDB doctor: {status}\nlocale: {}\ndrivers: postgres, mysql, sqlite",
                report.checks[0].status
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{CLI_API_VERSION, Cli, Command, capabilities, render_command};

    #[test]
    fn parses_direct_connection_url() {
        let cli =
            Cli::try_parse_from(["lazydb", "--url", "sqlite://demo.db", "--read-only"]).unwrap();

        assert_eq!(cli.url.as_deref(), Some("sqlite://demo.db"));
        assert!(cli.read_only);
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
            serde_json::json!(["mouse", "read-only", "context-help"])
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
}
