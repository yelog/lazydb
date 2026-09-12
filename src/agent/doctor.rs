use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use crate::cli::McpClient;

#[derive(Debug, Serialize)]
struct Check {
    name: String,
    status: &'static str,
    detail: String,
}

#[derive(Debug, Serialize)]
struct ClientReport {
    client: String,
    config_path: PathBuf,
    status: &'static str,
    checks: Vec<Check>,
}

pub async fn run(
    clients: Vec<McpClient>,
    project: Option<PathBuf>,
    _config: Option<PathBuf>,
    probe: bool,
    json: bool,
) -> Result<String> {
    run_with_options(clients, project, None, probe, json).await
}

pub async fn run_with_options(
    clients: Vec<McpClient>,
    project: Option<PathBuf>,
    client_config: Option<PathBuf>,
    probe: bool,
    json: bool,
) -> Result<String> {
    let project = project.unwrap_or(std::env::current_dir()?).canonicalize()?;
    let clients = if clients.is_empty() {
        vec![McpClient::ClaudeCode, McpClient::Codex, McpClient::Opencode]
    } else {
        clients
    };
    if client_config.is_some() && clients.len() != 1 {
        anyhow::bail!("--client-config requires exactly one --client");
    }
    let locations = super::client_config::Locations::discover()?;
    let client_config = client_config.map(std::path::absolute).transpose()?;
    let reports = clients
        .into_iter()
        .map(|client| inspect_client(client, &project, &locations, client_config.as_deref()))
        .collect::<Vec<_>>();
    let mut warnings = vec!["database I/O was not performed".to_owned()];
    warnings.push("static file inspection only: client startup, project trust, remote/managed settings and CLI overrides are not verified".into());
    if std::env::var_os("OPENCODE_CONFIG_CONTENT").is_some() {
        warnings.push("OPENCODE_CONFIG_CONTENT is set and may override file configuration".into());
    }
    if probe {
        warnings.push("--probe is not implemented; no configured client was started".to_owned());
    }
    let failed = reports.iter().any(|report| report.status == "failed");
    let status = if failed {
        "failed"
    } else if reports.iter().any(|report| report.status == "warning") {
        "warning"
    } else {
        "ok"
    };
    if json {
        return Ok(serde_json::json!({
            "schema_version": 1,
            "status": status,
            "project": project,
            "clients": reports,
            "warnings": warnings,
        })
        .to_string());
    }
    let mut output = format!(
        "LazyDB MCP doctor: {status}\nproject: {}\ndatabase I/O: not performed",
        project.display()
    );
    for report in reports {
        output.push_str(&format!(
            "\n{}: {} ({})",
            report.client,
            report.status,
            report.config_path.display()
        ));
        for check in report.checks {
            output.push_str(&format!(
                "\n  {}: {} ({})",
                check.name, check.status, check.detail
            ));
        }
    }
    for warning in warnings {
        output.push_str(&format!("\nwarning: {warning}"));
    }
    Ok(output)
}

fn inspect_client(
    client: McpClient,
    project: &std::path::Path,
    locations: &super::client_config::Locations,
    explicit: Option<&std::path::Path>,
) -> ClientReport {
    use super::client_config as cfg;
    let mut sources = locations.sources(client, project);
    if let Some(path) = explicit {
        sources.retain(|s| s.path == path);
        if sources.is_empty() {
            sources.push(cfg::Source {
                path: path.into(),
                scope: crate::cli::McpScope::Project,
                origin: "explicit (loading not verified)".into(),
            });
        }
    }
    let mut report = ClientReport {
        client: super::setup::client_name(client).into(),
        config_path: sources.last().unwrap().path.clone(),
        status: "warning",
        checks: Vec::new(),
    };
    let mut effective = None;
    let mut count = 0;
    for source in sources {
        let result = (|| -> Result<Option<serde_json::Value>> {
            let Some(text) = cfg::read_optional(&source.path)? else {
                return Ok(None);
            };
            let value = cfg::parse(client, &text)?;
            let entry = cfg::effective_entry(client, &value, &source, project)?;
            Ok(entry.cloned())
        })();
        match result {
            Ok(Some(value)) => {
                count += 1;
                report.checks.push(Check {
                    name: "source".into(),
                    status: "ok",
                    detail: format!(
                        "{} ({:?}, {}) contains LazyDB",
                        source.path.display(),
                        source.scope,
                        source.origin
                    ),
                });
                report.config_path = source.path;
                if client == McpClient::ClaudeCode {
                    effective = Some(value);
                } else {
                    let base = effective.get_or_insert_with(|| serde_json::json!({}));
                    merge(base, value);
                }
            }
            Ok(None) => {}
            Err(error) => report.checks.push(Check {
                name: "configuration".into(),
                status: "failed",
                detail: format!("{}: {error}", source.path.display()),
            }),
        }
    }
    if let Some(value) = effective {
        let disabled = if client == McpClient::Opencode
            && value.get("disabled").and_then(|v| v.as_bool()) == Some(true)
        {
            true
        } else {
            value.get("enabled").and_then(|v| v.as_bool()) == Some(false)
        };
        let valid = valid_server(client, &value);
        report.status = if valid && !disabled { "ok" } else { "warning" };
        report.checks.push(Check {
            name: "configuration".into(),
            status: report.status,
            detail: if disabled {
                "LazyDB entry is disabled".into()
            } else if valid {
                "LazyDB server entry found with deny policy (static inspection)".into()
            } else {
                "LazyDB command/arguments or deny policy could not be confirmed".into()
            },
        });
    } else {
        report.checks.push(Check {
            name: "configuration".into(),
            status: "warning",
            detail: "missing LazyDB server entry in discovered configuration files".into(),
        });
    }
    if count > 1 {
        report.checks.push(Check { name: "precedence".into(), status: "warning", detail: "multiple LazyDB definitions: sources are listed from lower to higher file precedence; earlier fields may be shadowed".into() });
        if report.status == "ok" {
            report.status = "warning";
        }
    }
    if report.checks.iter().any(|c| c.status == "failed") {
        report.status = "failed";
    }
    report
}

fn merge(base: &mut serde_json::Value, value: serde_json::Value) {
    match (base, value) {
        (serde_json::Value::Object(base), serde_json::Value::Object(value)) => {
            for (key, value) in value {
                merge(base.entry(key).or_insert(serde_json::Value::Null), value);
            }
        }
        (base, value) => *base = value,
    }
}

fn valid_server(client: McpClient, value: &serde_json::Value) -> bool {
    let (command, args) = if client == McpClient::Opencode {
        if value.get("type").and_then(|v| v.as_str()) != Some("local") {
            return false;
        }
        let Some(command) = value.get("command").and_then(|v| v.as_array()) else {
            return false;
        };
        (command.first().and_then(|v| v.as_str()), command.get(1..))
    } else {
        if client == McpClient::ClaudeCode && value.get("type").is_some_and(|v| v != "stdio") {
            return false;
        }
        (
            value.get("command").and_then(|v| v.as_str()),
            value
                .get("args")
                .and_then(|v| v.as_array())
                .map(|v| v.as_slice()),
        )
    };
    let binary = command
        .and_then(|c| std::path::Path::new(c).file_name())
        .and_then(|n| n.to_str());
    let Some(args) = args else {
        return false;
    };
    matches!(binary, Some("lazydb" | "lazydb.exe"))
        && args.windows(2).any(|a| a[0] == "mcp" && a[1] == "serve")
        && args
            .windows(2)
            .rfind(|a| a[0] == "--write-policy")
            .is_some_and(|a| a[1] == "deny")
}
