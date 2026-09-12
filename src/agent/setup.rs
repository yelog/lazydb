use super::client_config::{self as cfg, Locations, Source};
use crate::cli::{McpClient, McpScope};
use anyhow::{Context, Result, bail};
use std::{
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
};

pub struct SetupOptions {
    pub clients: Vec<McpClient>,
    pub scope: Option<McpScope>,
    pub client_config: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub dry_run: bool,
    pub yes: bool,
    pub json: bool,
}

/// Compatibility entry point for callers that explicitly select a scope.
pub fn run(
    clients: Vec<McpClient>,
    scope: McpScope,
    project: Option<PathBuf>,
    config: Option<PathBuf>,
    dry_run: bool,
    yes: bool,
    json: bool,
) -> Result<String> {
    run_with_options(SetupOptions {
        clients,
        scope: Some(scope),
        client_config: None,
        project,
        config,
        dry_run,
        yes,
        json,
    })
}

pub fn run_with_options(mut options: SetupOptions) -> Result<String> {
    if options.json && !options.yes && !options.dry_run {
        bail!("--json setup requires --yes or --dry-run");
    }
    let interactive = std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && !options.json
        && !options.yes;
    if options.clients.is_empty() {
        if !interactive {
            bail!("at least one --client is required in non-interactive mode");
        }
        println!("Configure LazyDB MCP: [1] Claude Code [2] Codex [3] OpenCode");
        for number in prompt("Select clients (for example 1,2): ")?
            .split(',')
            .map(str::trim)
        {
            let client = match number {
                "1" => McpClient::ClaudeCode,
                "2" => McpClient::Codex,
                "3" => McpClient::Opencode,
                _ => bail!("invalid client selection"),
            };
            if !options.clients.contains(&client) {
                options.clients.push(client);
            }
        }
    }
    let mut unique = Vec::new();
    options.clients.retain(|client| {
        if unique.contains(client) {
            false
        } else {
            unique.push(*client);
            true
        }
    });
    if options.client_config.is_some() && options.clients.len() != 1 {
        bail!("--client-config requires exactly one --client");
    }
    let project = options
        .project
        .clone()
        .unwrap_or(std::env::current_dir()?)
        .canonicalize()?;
    if interactive {
        println!(
            "Project directory for project/local MCP scopes: {}",
            project.display()
        );
    }
    let config = options
        .config
        .as_ref()
        .map(|p| p.canonicalize())
        .transpose()
        .context("cannot resolve LazyDB --config")?;
    let locations = Locations::discover()?;
    let mut plans = Vec::new();
    for client in options.clients.iter().copied() {
        let sources = locations.sources(client, &project);
        let target = select_target(client, &sources, &project, &options, interactive)?;
        plans.push(plan_client(
            client,
            target,
            &project,
            config.as_deref(),
            &sources,
        ));
    }
    let actionable = plans.iter().any(|p| matches!(p.status, "create" | "add"));
    let confirmed = if !actionable || options.yes || options.dry_run {
        true
    } else {
        if !interactive {
            bail!("non-interactive setup requires --yes or --dry-run");
        }
        println!("Planned MCP configuration changes:");
        for plan in &plans {
            println!(
                "  {}: {} ({}) — {}",
                plan.client,
                plan.status,
                plan.config_path.display(),
                plan.message
            );
        }
        matches!(
            prompt("Continue? [y/N] ")?.to_ascii_lowercase().as_str(),
            "y" | "yes"
        )
    };
    if confirmed && !options.dry_run {
        for plan in &mut plans {
            if matches!(plan.status, "create" | "add") {
                cfg::write(&plan.config_path, plan.original.as_deref(), &plan.content)?;
                plan.status = if plan.status == "create" {
                    "created"
                } else {
                    "added"
                };
            }
        }
    }
    let status = if options.dry_run {
        "dry_run"
    } else if plans
        .iter()
        .any(|p| matches!(p.status, "conflict" | "invalid"))
    {
        "warning"
    } else if confirmed {
        "complete"
    } else {
        "planned"
    };
    if options.json {
        return Ok(serde_json::json!({"schema_version": 1, "status": status, "project": project, "clients": plans,
            "warnings": ["MCP write policy defaults to deny", "global LazyDB profiles remain visible to the project server", "static configuration inspection; client startup and trust are not verified"]}).to_string());
    }
    let mut output = format!(
        "MCP setup {status} for {}\nwrite policy: deny",
        project.display()
    );
    for plan in plans {
        output.push_str(&format!(
            "\n{}: {} ({}, {:?}) — {}",
            plan.client,
            plan.status,
            plan.config_path.display(),
            plan.scope,
            plan.message
        ));
        for note in plan.notes {
            output.push_str(&format!("\n  {note}"));
        }
    }
    Ok(output)
}

fn prompt(message: &str) -> Result<String> {
    print!("{message}");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_owned())
}

fn select_target(
    client: McpClient,
    sources: &[Source],
    project: &Path,
    options: &SetupOptions,
    interactive: bool,
) -> Result<Source> {
    if options.scope == Some(McpScope::Local) && client != McpClient::ClaudeCode {
        bail!("local scope is supported only by Claude Code");
    }
    if let Some(path) = &options.client_config {
        let path = std::path::absolute(path)?;
        let known = sources
            .iter()
            .find(|s| std::path::absolute(&s.path).ok().as_ref() == Some(&path));
        return Ok(Source {
            path,
            scope: options.scope.or(known.map(|s| s.scope)).unwrap_or_default(),
            origin: "--client-config".into(),
        });
    }
    let mut candidates: Vec<_> = sources
        .iter()
        .filter(|s| options.scope.is_none_or(|scope| scope == s.scope) && s.path.exists())
        .cloned()
        .collect();
    if interactive && options.scope.is_none() {
        // Existing server entries first, then existing user configuration.
        candidates.sort_by_key(|s| {
            let registered = cfg::read_optional(&s.path)
                .ok()
                .flatten()
                .and_then(|text| cfg::parse(client, &text).ok())
                .is_some_and(|v| {
                    cfg::entry(&v, &cfg::keys(client, s, project))
                        .ok()
                        .flatten()
                        .is_some()
                });
            (!registered, s.scope != McpScope::User)
        });
        for scope in [McpScope::User, McpScope::Project] {
            if !candidates.iter().any(|s| s.scope == scope)
                && let Some(source) = sources.iter().find(|s| s.scope == scope)
            {
                candidates.push(source.clone());
            }
        }
        if client == McpClient::ClaudeCode && !candidates.iter().any(|s| s.scope == McpScope::Local)
        {
            candidates.push(
                sources
                    .iter()
                    .find(|s| s.scope == McpScope::Local)
                    .unwrap()
                    .clone(),
            );
        }
        println!("{} configuration targets:", client_name(client));
        for (i, source) in candidates.iter().enumerate() {
            println!(
                "  [{}] {:?}: {} ({}){}",
                i + 1,
                source.scope,
                source.path.display(),
                if source.path.exists() {
                    "exists"
                } else {
                    "new file"
                },
                if i == 0 { " — recommended" } else { "" }
            );
        }
        let answer = prompt("Select target [1]: ")?;
        let index = if answer.is_empty() {
            1
        } else {
            answer.parse::<usize>()?
        };
        return candidates
            .get(index.wrapping_sub(1))
            .cloned()
            .context("invalid target selection");
    }
    let scope = options.scope.unwrap_or_default();
    candidates.retain(|s| s.scope == scope);
    // A scope does not authorize arbitrarily choosing between multiple existing files.
    if candidates.len() > 1 {
        bail!(
            "multiple {:?} configurations for {}; select one with --client-config: {}",
            scope,
            client_name(client),
            candidates
                .iter()
                .map(|s| s.path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    candidates
        .pop()
        .or_else(|| sources.iter().find(|s| s.scope == scope).cloned())
        .context("no configuration target for scope")
}

#[derive(Debug, serde::Serialize)]
struct ClientPlan {
    client: String,
    status: &'static str,
    config_path: PathBuf,
    scope: McpScope,
    message: String,
    discovered: Vec<Source>,
    notes: Vec<String>,
    #[serde(skip)]
    original: Option<String>,
    #[serde(skip)]
    content: String,
}

fn plan_client(
    client: McpClient,
    target: Source,
    project: &Path,
    config: Option<&Path>,
    sources: &[Source],
) -> ClientPlan {
    let mut plan = ClientPlan {
        client: client_name(client).into(),
        status: "invalid",
        config_path: target.path.clone(),
        scope: target.scope,
        message: String::new(),
        discovered: sources
            .iter()
            .filter(|s| s.path.exists())
            .cloned()
            .collect(),
        notes: Vec::new(),
        original: None,
        content: String::new(),
    };
    let result = (|| -> Result<()> {
        plan.original = cfg::read_optional(&target.path)?;
        let text = plan
            .original
            .as_deref()
            .unwrap_or(if client == McpClient::Codex {
                ""
            } else {
                "{\n}\n"
            });
        let value = cfg::parse(client, text)?;
        let keys = if client == McpClient::Opencode {
            cfg::insert_keys(client, &value)
        } else {
            cfg::keys(client, &target, project)
        };
        let desired = cfg::desired(client, config);
        let existing = if client == McpClient::Opencode {
            cfg::effective_entry(client, &value, &target, project)?
        } else {
            cfg::entry(&value, &keys)?
        };
        if let Some(existing) = existing {
            if cfg::equivalent(existing, &desired) {
                plan.status = "unchanged";
                plan.message = "LazyDB is already configured; no changes".into();
            } else {
                plan.status = "conflict";
                let fields = desired
                    .as_object()
                    .unwrap()
                    .iter()
                    .filter(|(key, value)| existing.get(*key) != Some(*value))
                    .map(|(key, _)| key.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                plan.message = format!(
                    "existing LazyDB entry differs in {fields}; review manually (not overwritten by --yes)"
                );
            }
            if existing.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                plan.notes
                    .push("LazyDB is disabled in this configuration".into());
            }
        } else {
            plan.content = cfg::insert(client, text, &keys, &desired)?;
            cfg::parse(client, &plan.content)?;
            plan.status = if plan.original.is_some() {
                "add"
            } else {
                "create"
            };
            plan.message = format!("register {}", keys.join("."));
        }
        Ok(())
    })();
    if let Err(error) = result {
        plan.status = "invalid";
        plan.message = error.to_string();
    }
    for source in &plan.discovered {
        if source.path == target.path && source.scope == target.scope {
            continue;
        }
        if cfg::read_optional(&source.path)
            .ok()
            .flatten()
            .and_then(|text| cfg::parse(client, &text).ok())
            .is_some_and(|value| {
                cfg::entry(&value, &cfg::keys(client, source, project))
                    .ok()
                    .flatten()
                    .is_some()
            })
        {
            plan.notes.push(format!(
                "another LazyDB entry exists in {} ({:?}); inspect precedence with mcp doctor",
                source.path.display(),
                source.scope
            ));
        }
    }
    if client == McpClient::Codex && target.scope == McpScope::Project {
        plan.notes
            .push("Codex loads project configuration only for trusted projects".into());
    }
    if target.origin == "--client-config" {
        plan.notes
            .push("explicit file selected; ensure the client loads this path".into());
    }
    plan
}

pub(crate) fn client_name(client: McpClient) -> &'static str {
    match client {
        McpClient::ClaudeCode => "claude-code",
        McpClient::Codex => "codex",
        McpClient::Opencode => "opencode",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_user_config_is_selected_and_project_file_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("home/.config/opencode/opencode.json");
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        std::fs::write(&user, "{\"model\":\"custom\"}").unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let locations = Locations {
            home: dir.path().join("home"),
            ..Default::default()
        };
        let sources = locations.sources(McpClient::Opencode, &project);
        let options = SetupOptions {
            clients: vec![McpClient::Opencode],
            scope: Some(McpScope::User),
            client_config: None,
            project: Some(project.clone()),
            config: None,
            dry_run: true,
            yes: false,
            json: true,
        };
        let target =
            select_target(McpClient::Opencode, &sources, &project, &options, false).unwrap();
        assert_eq!(target.path, user);
        let plan = plan_client(McpClient::Opencode, target, &project, None, &sources);
        assert_eq!(plan.status, "add");
        assert!(!project.join("opencode.json").exists());
        assert_eq!(
            std::fs::read_to_string(user).unwrap(),
            "{\"model\":\"custom\"}"
        );
    }
}
