use std::{fs, path::PathBuf};

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
    let project = project.unwrap_or(std::env::current_dir()?).canonicalize()?;
    let clients = if clients.is_empty() {
        vec![McpClient::ClaudeCode, McpClient::Codex, McpClient::Opencode]
    } else {
        clients
    };
    let reports = clients
        .into_iter()
        .map(|client| inspect_client(client, &project))
        .collect::<Vec<_>>();
    let mut warnings = vec!["database I/O was not performed".to_owned()];
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
    if probe {
        output.push_str("\nprobe: not implemented; no configured client was started");
    }
    Ok(output)
}

fn inspect_client(client: McpClient, project: &std::path::Path) -> ClientReport {
    let (relative, required) = match client {
        McpClient::ClaudeCode => (PathBuf::from(".mcp.json"), "mcpServers"),
        McpClient::Codex => (PathBuf::from(".codex/config.toml"), "mcp_servers.lazydb"),
        McpClient::Opencode => {
            let path = if project.join("opencode.jsonc").exists() {
                "opencode.jsonc"
            } else {
                "opencode.json"
            };
            (PathBuf::from(path), "mcp.lazydb")
        }
    };
    let config_path = project.join(relative);
    if !config_path.exists() {
        return ClientReport {
            client: super::setup::client_name(client).to_owned(),
            config_path,
            status: "warning",
            checks: vec![Check {
                name: "configuration".to_owned(),
                status: "warning",
                detail: format!("missing; expected {required}"),
            }],
        };
    }
    let contents = match fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(error) => {
            return ClientReport {
                client: super::setup::client_name(client).to_owned(),
                config_path,
                status: "failed",
                checks: vec![Check {
                    name: "configuration".to_owned(),
                    status: "failed",
                    detail: format!("cannot read configuration: {error}"),
                }],
            };
        }
    };
    let contains_server = contents.contains("lazydb")
        && contents.contains("write-policy")
        && contents.contains("deny");
    ClientReport {
        client: super::setup::client_name(client).to_owned(),
        config_path,
        status: if contains_server { "ok" } else { "warning" },
        checks: vec![Check {
            name: "configuration".to_owned(),
            status: if contains_server { "ok" } else { "warning" },
            detail: if contains_server {
                "LazyDB server entry found with deny policy".to_owned()
            } else {
                format!("file exists but {required} LazyDB entry was not confirmed")
            },
        }],
    }
}
