use std::{
    fs,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

use crate::cli::{McpClient, McpScope};

pub fn run(
    clients: Vec<McpClient>,
    scope: McpScope,
    project: Option<PathBuf>,
    config: Option<PathBuf>,
    dry_run: bool,
    yes: bool,
    json: bool,
) -> Result<String> {
    if scope != McpScope::Project {
        bail!("only project-scoped MCP configuration is supported");
    }
    if json && !yes && !dry_run {
        bail!("--json setup requires --yes or --dry-run");
    }
    let clients = if clients.is_empty() {
        interactive_clients()?
    } else {
        clients
    };
    let project = project.unwrap_or(std::env::current_dir()?).canonicalize()?;
    let mut plans = clients
        .iter()
        .copied()
        .map(|client| plan_client(client, &project, config.as_deref()))
        .collect::<Result<Vec<_>>>()?;

    let confirmed = yes || dry_run || (!json && confirm_write(&plans)?);
    if confirmed && !dry_run {
        for plan in &mut plans {
            if plan.status == "create" {
                write_new_config(plan)?;
                plan.status = "created";
            }
        }
    }

    let has_conflict = plans.iter().any(|plan| plan.status == "conflict");
    let status = if dry_run {
        "dry_run"
    } else if has_conflict {
        "warning"
    } else if confirmed {
        "complete"
    } else {
        "planned"
    };
    if json {
        return Ok(serde_json::json!({
            "schema_version": 1,
            "status": status,
            "project": project,
            "clients": plans,
            "warnings": [
                "MCP write policy defaults to deny",
                "global LazyDB profiles remain visible to the project server"
            ]
        })
        .to_string());
    }
    Ok(format!(
        "MCP setup {status} for {}\n{}\nwrite policy: deny",
        project.display(),
        plans
            .iter()
            .map(|plan| format!(
                "{}: {} ({})",
                plan.client,
                plan.status,
                plan.config_path.display()
            ))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn interactive_clients() -> Result<Vec<McpClient>> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("at least one --client is required in non-interactive mode");
    }
    println!("Configure LazyDB MCP for which clients? [1] Claude Code [2] Codex [3] OpenCode");
    print!("Select one or more numbers (for example 1,2): ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let mut clients = Vec::new();
    for value in input.trim().split(',').map(str::trim) {
        let client = match value {
            "1" => McpClient::ClaudeCode,
            "2" => McpClient::Codex,
            "3" => McpClient::Opencode,
            "" => continue,
            _ => bail!("invalid client selection: {value}"),
        };
        if !clients.contains(&client) {
            clients.push(client);
        }
    }
    if clients.is_empty() {
        bail!("at least one client must be selected");
    }
    Ok(clients)
}

fn confirm_write(plans: &[ClientPlan]) -> Result<bool> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("non-interactive setup requires --yes or --dry-run");
    }
    println!("The following new project configuration files will be created:");
    for plan in plans.iter().filter(|plan| plan.status == "create") {
        println!("  {}", plan.config_path.display());
    }
    print!("Continue? [y/N] ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[derive(Debug, serde::Serialize)]
struct ClientPlan {
    client: String,
    status: &'static str,
    config_path: PathBuf,
    message: &'static str,
    #[serde(skip)]
    content: String,
}

fn plan_client(client: McpClient, project: &Path, config: Option<&Path>) -> Result<ClientPlan> {
    let (relative, message, content) = match client {
        McpClient::ClaudeCode => (
            PathBuf::from(".mcp.json"),
            "new file only; merge existing files manually",
            claude_config(config),
        ),
        McpClient::Codex => (
            PathBuf::from(".codex/config.toml"),
            "new file only; merge existing files manually",
            codex_config(config),
        ),
        McpClient::Opencode => {
            let relative = if project.join("opencode.jsonc").exists() {
                PathBuf::from("opencode.jsonc")
            } else {
                PathBuf::from("opencode.json")
            };
            (
                relative,
                "new file only; merge existing files manually",
                opencode_config(config),
            )
        }
    };
    let config_path = project.join(relative);
    let status = if config_path.exists() {
        "conflict"
    } else {
        "create"
    };
    Ok(ClientPlan {
        client: client_name(client).to_owned(),
        status,
        config_path,
        message,
        content,
    })
}

fn server_args(config: Option<&Path>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(config) = config {
        args.extend(["--config".to_owned(), config.display().to_string()]);
    }
    args.extend(["mcp", "serve", "--project", ".", "--write-policy", "deny"].map(str::to_owned));
    args
}

fn claude_config(config: Option<&Path>) -> String {
    let args = server_args(config);
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": { "lazydb": { "type": "stdio", "command": "lazydb", "args": args } }
    }))
    .expect("static configuration serializes")
        + "\n"
}

fn opencode_config(config: Option<&Path>) -> String {
    let args = server_args(config);
    serde_json::to_string_pretty(&serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "mcp": { "lazydb": { "type": "local", "command": std::iter::once("lazydb".to_owned()).chain(args).collect::<Vec<_>>(), "cwd": "." } }
    })).expect("static configuration serializes") + "\n"
}

fn codex_config(config: Option<&Path>) -> String {
    let args = server_args(config)
        .into_iter()
        .map(|arg| format!("\"{arg}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[mcp_servers.lazydb]\ncommand = \"lazydb\"\nargs = [{args}]\nrequired = true\n")
}

fn write_new_config(plan: &ClientPlan) -> Result<()> {
    if plan.config_path.exists() {
        bail!(
            "refusing to overwrite existing MCP configuration: {}",
            plan.config_path.display()
        );
    }
    if let Some(parent) = plan.config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = plan.config_path.with_extension("lazydb.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(plan.content.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, &plan.config_path)?;
    Ok(())
}

pub(crate) fn client_name(client: McpClient) -> &'static str {
    match client {
        McpClient::ClaudeCode => "claude-code",
        McpClient::Codex => "codex",
        McpClient::Opencode => "opencode",
    }
}
