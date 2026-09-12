//! Client-specific configuration locations and lossless server registration.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstRootNode},
};
use serde_json::{Value, json};

use crate::cli::{McpClient, McpScope};

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct Source {
    pub path: PathBuf,
    pub scope: McpScope,
    pub origin: String,
}

#[derive(Default)]
pub(crate) struct Locations {
    pub home: PathBuf,
    pub xdg: Option<PathBuf>,
    pub codex: Option<PathBuf>,
    pub claude: Option<PathBuf>,
    pub opencode: Option<PathBuf>,
    pub opencode_dir: Option<PathBuf>,
}

impl Locations {
    pub fn discover() -> Result<Self> {
        let env = |name| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Ok(Self {
            home: directories::BaseDirs::new()
                .context("cannot discover home directory")?
                .home_dir()
                .into(),
            xdg: env("XDG_CONFIG_HOME"),
            codex: env("CODEX_HOME"),
            claude: env("CLAUDE_CONFIG_DIR"),
            opencode: env("OPENCODE_CONFIG"),
            opencode_dir: env("OPENCODE_CONFIG_DIR"),
        })
    }

    pub fn sources(&self, client: McpClient, project: &Path) -> Vec<Source> {
        let mut sources = Vec::new();
        let mut add = |path: PathBuf, scope, origin: &str| {
            if !sources
                .iter()
                .any(|s: &Source| s.path == path && s.scope == scope)
            {
                sources.push(Source {
                    path,
                    scope,
                    origin: origin.into(),
                });
            }
        };
        let claude = self
            .claude
            .as_ref()
            .map(|p| p.join(".claude.json"))
            .unwrap_or_else(|| self.home.join(".claude.json"));
        match client {
            McpClient::ClaudeCode => add(claude.clone(), McpScope::User, "user"),
            McpClient::Codex => add(
                self.codex
                    .clone()
                    .unwrap_or_else(|| self.home.join(".codex"))
                    .join("config.toml"),
                McpScope::User,
                "user",
            ),
            McpClient::Opencode => {
                let dir = self
                    .xdg
                    .clone()
                    .unwrap_or_else(|| self.home.join(".config"))
                    .join("opencode");
                for name in ["opencode.json", "opencode.jsonc"] {
                    add(dir.join(name), McpScope::User, "user");
                }
                if let Some(path) = &self.opencode {
                    add(path.clone(), McpScope::User, "OPENCODE_CONFIG");
                }
            }
        }
        let mut ancestors = Vec::new();
        for dir in project.ancestors() {
            ancestors.push(dir);
            if dir.join(".git").exists() {
                break;
            }
        }
        // Without a repository boundary, do not treat arbitrary ancestors as a project.
        if !ancestors.last().is_some_and(|p| p.join(".git").exists()) {
            ancestors.truncate(1);
        }
        for dir in ancestors.iter().rev() {
            match client {
                McpClient::ClaudeCode => {}
                McpClient::Codex => add(
                    dir.join(".codex/config.toml"),
                    McpScope::Project,
                    "project (requires trust)",
                ),
                McpClient::Opencode => {
                    for name in ["opencode.json", "opencode.jsonc"] {
                        add(dir.join(name), McpScope::Project, "project");
                    }
                }
            }
        }
        if client == McpClient::Opencode {
            for dir in ancestors.iter().rev() {
                for name in ["opencode.json", "opencode.jsonc"] {
                    add(
                        dir.join(".opencode").join(name),
                        McpScope::Project,
                        ".opencode",
                    );
                }
            }
            if let Some(dir) = &self.opencode_dir {
                for name in ["opencode.json", "opencode.jsonc"] {
                    add(dir.join(name), McpScope::User, "OPENCODE_CONFIG_DIR");
                }
            }
        }
        if client == McpClient::ClaudeCode {
            add(
                ancestors.last().unwrap_or(&project).join(".mcp.json"),
                McpScope::Project,
                "project",
            );
            add(claude, McpScope::Local, "local");
        }
        sources
    }
}

pub(crate) fn keys(client: McpClient, source: &Source, project: &Path) -> Vec<String> {
    let mut keys = Vec::new();
    if client == McpClient::ClaudeCode && source.scope == McpScope::Local {
        keys.extend(["projects".into(), project.to_string_lossy().into_owned()]);
    }
    keys.extend([
        match client {
            McpClient::ClaudeCode => "mcpServers",
            McpClient::Codex => "mcp_servers",
            McpClient::Opencode => "mcp",
        }
        .into(),
        "lazydb".into(),
    ]);
    keys
}

/// Find the effective LazyDB entry without making callers know about the
/// OpenCode V1/V2 layout. Other clients retain their existing key semantics.
pub(crate) fn effective_entry<'a>(
    client: McpClient,
    value: &'a Value,
    source: &Source,
    project: &Path,
) -> Result<Option<&'a Value>> {
    if client == McpClient::Opencode {
        let Some(format) = crate::agent::opencode_config::effective_format(value)? else {
            return Ok(None);
        };
        let keys = format
            .path()
            .iter()
            .map(|key| (*key).to_owned())
            .collect::<Vec<_>>();
        return entry(value, &keys);
    }
    entry(value, &keys(client, source, project))
}

/// Return the key path to use when adding a new OpenCode entry.
pub(crate) fn insert_keys(client: McpClient, value: &Value) -> Vec<String> {
    if client != McpClient::Opencode {
        return vec!["mcp".into(), "lazydb".into()];
    }
    if value
        .get("mcp")
        .and_then(Value::as_object)
        .and_then(|mcp| mcp.get("servers"))
        .is_some()
    {
        vec!["mcp".into(), "servers".into(), "lazydb".into()]
    } else {
        vec!["mcp".into(), "lazydb".into()]
    }
}

pub(crate) fn parse(client: McpClient, text: &str) -> Result<Value> {
    if client == McpClient::Codex {
        let value: toml::Value =
            toml::from_str(text).map_err(|_| anyhow::anyhow!("invalid TOML configuration"))?;
        return Ok(serde_json::to_value(value)?);
    }
    if client == McpClient::ClaudeCode {
        return serde_json::from_str(text)
            .map_err(|_| anyhow::anyhow!("invalid JSON configuration"));
    }
    CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|_| anyhow::anyhow!("invalid JSONC configuration"))?
        .to_serde_value()
        .context("configuration must contain an object")
}

pub(crate) fn entry<'a>(value: &'a Value, keys: &[String]) -> Result<Option<&'a Value>> {
    let mut node = value;
    for key in keys {
        let object = node
            .as_object()
            .context("configuration parent must be an object")?;
        match object.get(key) {
            Some(value) => node = value,
            None => return Ok(None),
        }
    }
    if !node.is_object() {
        bail!("LazyDB entry must be an object");
    }
    Ok(Some(node))
}

pub(crate) fn desired(client: McpClient, config: Option<&Path>) -> Value {
    let mut args = Vec::<String>::new();
    if let Some(path) = config {
        args.extend(["--config".into(), path.to_string_lossy().into_owned()]);
    }
    args.extend(["mcp", "serve"].map(str::to_owned));
    // Claude supplies CLAUDE_PROJECT_DIR to the child process. Let serve use it
    // instead of assuming the process working directory is the workspace.
    if client != McpClient::ClaudeCode {
        args.extend(["--project", "."].map(str::to_owned));
    }
    args.extend(["--write-policy", "deny"].map(str::to_owned));
    match client {
        McpClient::Opencode => {
            json!({"type":"local", "command": std::iter::once("lazydb".to_owned()).chain(args).collect::<Vec<_>>(), "cwd":"."})
        }
        McpClient::ClaudeCode => json!({"type":"stdio", "command":"lazydb", "args":args}),
        McpClient::Codex => json!({"command":"lazydb", "args":args}),
    }
}

pub(crate) fn equivalent(existing: &Value, desired: &Value) -> bool {
    desired.as_object().is_some_and(|expected| {
        expected.iter().all(|(key, value)| {
            existing.get(key) == Some(value)
                || (key == "type" && value == "stdio" && existing.get(key).is_none())
        })
    })
}

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Object(map) => {
            CstInputValue::Object(map.iter().map(|(k, v)| (k.clone(), input(v))).collect())
        }
        Value::Array(items) => CstInputValue::Array(items.iter().map(input).collect()),
        Value::String(s) => s.clone().into(),
        Value::Bool(v) => (*v).into(),
        _ => unreachable!("generated server values are objects, arrays, strings or booleans"),
    }
}

pub(crate) fn insert(
    client: McpClient,
    text: &str,
    keys: &[String],
    value: &Value,
) -> Result<String> {
    if client == McpClient::Codex {
        let mut doc = text.parse::<toml_edit::DocumentMut>()?;
        let table = doc
            .entry("mcp_servers")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        let table = table
            .as_table_like_mut()
            .context("mcp_servers must be a table")?;
        let mut server = toml_edit::Table::new();
        server.insert("command", toml_edit::value("lazydb"));
        let mut args = toml_edit::Array::new();
        for arg in value["args"].as_array().unwrap() {
            args.push(arg.as_str().unwrap());
        }
        server.insert("args", toml_edit::value(args));
        table.insert("lazydb", toml_edit::Item::Table(server));
        return Ok(doc.to_string());
    }
    let root = CstRootNode::parse(text, &ParseOptions::default())?;
    let mut object = root
        .object_value()
        .context("configuration must be an object")?;
    for key in &keys[..keys.len() - 1] {
        object = object
            .object_value_or_create(key)
            .context("configuration parent must be an object")?;
    }
    object.append(keys.last().unwrap(), input(value));
    Ok(root.to_string())
}

pub(crate) fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok() {
                bail!("configuration is a dangling symlink");
            }
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn write(path: &Path, original: Option<&str>, content: &str) -> Result<()> {
    // Resolve existing symlinks so atomic replacement preserves the user's link.
    let target = if original.is_some() {
        path.canonicalize()?
    } else {
        path.to_owned()
    };
    if read_optional(&target)?.as_deref() != original {
        bail!("configuration changed since preview: {}", path.display());
    }
    let parent = target
        .parent()
        .context("configuration has no parent directory")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".lazydb-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        if original.is_some() {
            file.set_permissions(fs::metadata(&target)?.permissions())?;
        }
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        if read_optional(&target)?.as_deref() != original {
            bail!("configuration changed since preview: {}", path.display());
        }
        if original.is_none() {
            // Linking is atomic and fails rather than replacing a concurrently created file.
            fs::hard_link(&temporary, &target)?;
            fs::remove_file(&temporary)?;
        } else {
            fs::rename(&temporary, &target)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn discovers_user_overrides_and_repository_layers() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let repo = dir.path().join("repo");
        let nested = repo.join("src/nested");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(&nested).unwrap();
        let locations = Locations {
            home,
            xdg: Some(dir.path().join("xdg")),
            codex: Some(dir.path().join("codex-home")),
            claude: Some(dir.path().join("claude-home")),
            opencode: Some(dir.path().join("custom.jsonc")),
            opencode_dir: Some(dir.path().join("custom-dir")),
        };
        let sources = locations.sources(McpClient::Opencode, &nested);
        assert_eq!(
            sources[0].path,
            dir.path().join("xdg/opencode/opencode.json")
        );
        assert!(
            sources
                .iter()
                .any(|s| s.path == repo.join("opencode.jsonc"))
        );
        assert!(
            sources
                .iter()
                .any(|s| s.path == dir.path().join("custom.jsonc"))
        );
        assert!(
            sources
                .iter()
                .any(|s| s.path == dir.path().join("custom-dir/opencode.jsonc"))
        );
        assert!(
            !sources
                .iter()
                .any(|s| s.path == dir.path().join("opencode.json"))
        );
        let sources = locations.sources(McpClient::Codex, &nested);
        assert_eq!(sources[0].path, dir.path().join("codex-home/config.toml"));
        assert_eq!(sources[1].path, repo.join(".codex/config.toml"));
        assert_eq!(
            sources.last().unwrap().path,
            nested.join(".codex/config.toml")
        );
        let sources = locations.sources(McpClient::ClaudeCode, &nested);
        assert_eq!(sources[0].path, dir.path().join("claude-home/.claude.json"));
        assert_eq!(sources[1].path, repo.join(".mcp.json"));
        assert_eq!(sources[2].scope, McpScope::Local);
    }

    #[test]
    fn toml_escapes_config_arguments() {
        let source = Source {
            path: "config.toml".into(),
            scope: McpScope::User,
            origin: "test".into(),
        };
        let desired = desired(
            McpClient::Codex,
            Some(Path::new("C:\\Users\\a\"b\\config.toml")),
        );
        let result = insert(
            McpClient::Codex,
            "",
            &keys(McpClient::Codex, &source, Path::new(".")),
            &desired,
        )
        .unwrap();
        assert_eq!(
            parse(McpClient::Codex, &result).unwrap()["mcp_servers"]["lazydb"],
            desired
        );
    }

    #[test]
    fn rejects_changed_files_before_writing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "new user edit").unwrap();
        assert!(write(&path, Some("old contents"), "generated").is_err());
        assert!(write(&path, None, "generated").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "new user edit");
    }

    #[test]
    #[cfg(unix)]
    fn preserves_symlinks_and_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("config.json");
        fs::write(&target, "{}").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &link).unwrap();
        write(&link, Some("{}"), "{\"updated\":true}").unwrap();
        assert!(fs::symlink_metadata(link).unwrap().is_symlink());
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "{\"updated\":true}");
    }
}
