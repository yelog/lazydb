# Coding Agent Database Access

LazyDB exposes project-aware database capabilities through a JSON CLI and a
local stdio MCP server. Both interfaces resolve connections from the current
canonical Git project.

## Visibility And Selection

Agent-visible connections are:

```text
current-project profiles + global profiles
```

Profiles assigned only to another project are hidden and cannot be selected by
name or UUID. A single current-project profile is selected before a global
profile. If multiple candidates exist, pass `--connection`; LazyDB never uses
storage order as a selection rule.

Global connections are intentionally visible to agents. Visibility does not
override `read_only`, database grants, or the MCP server write policy.

## CLI

```bash
lazydb agent connections --project .
lazydb agent context --project .
lazydb agent schema-search users --project . --connection orders-dev
lazydb agent query --project . --connection orders-dev --sql 'SELECT * FROM users LIMIT 20'
lazydb agent describe users --project . --connection orders-dev
```

SQL files must be below the explicitly supplied project path:

```bash
lazydb agent execute --project . --connection orders-dev \
  --file db/migrations/001.sql --write-policy non-production
```

Every successful command returns JSON. Diagnostics use stderr. Passwords,
credential references, and encrypted credential payloads are never returned.

## Permission Layers

| Layer | Responsibility |
| --- | --- |
| Database role and grants | Final data and write authority |
| LazyDB profile and MCP `--write-policy` | Server-side operation ceiling |
| Agent MCP permission configuration | Tool visibility and human approval |

MCP annotations and `AGENTS.md` are guidance, not authorization. Use a real
read-only database role for read-only profiles. `--write-policy deny` is the
default; `non-production` permits writable development/staging profiles; `all`
is required before a production write can be attempted.

## Codex

Create `.codex/config.toml`:

```toml
[mcp_servers.lazydb]
command = "lazydb"
args = ["mcp", "serve", "--project", ".", "--write-policy", "deny"]
required = true
enabled_tools = ["get_context", "list_connections", "search_schema", "describe_object", "query"]
```

For a development-only writable session, explicitly use
`--write-policy non-production`, enable write tools, and configure them to
require approval. Never put passwords in this file.

## OpenCode

Create or merge the project `opencode.json`:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "lazydb": {
      "type": "local",
      "command": ["lazydb", "mcp", "serve", "--project", ".", "--write-policy", "deny"],
      "cwd": "."
    }
  },
  "permission": {
    "lazydb_get_context": "allow",
    "lazydb_list_connections": "allow",
    "lazydb_search_schema": "allow",
    "lazydb_describe_object": "allow",
    "lazydb_query": "allow",
    "lazydb_execute_change": "ask",
    "lazydb_execute_file": "ask"
  }
}
```

OpenCode merges global and project configuration. A project LazyDB MCP does not
remove unrelated global database MCP servers; disable obsolete global servers
if they expose duplicate query tools.

## Claude Code

Create `.mcp.json`:

```json
{
  "mcpServers": {
    "lazydb": {
      "type": "stdio",
      "command": "lazydb",
      "args": ["mcp", "serve", "--project", ".", "--write-policy", "deny"]
    }
  }
}
```

Use `.claude/settings.json` for client-side approval:

```json
{
  "permissions": {
    "allow": ["mcp__lazydb__get_context", "mcp__lazydb__list_connections", "mcp__lazydb__search_schema", "mcp__lazydb__describe_object", "mcp__lazydb__query"],
    "ask": ["mcp__lazydb__execute_change", "mcp__lazydb__execute_file"]
  }
}
```

For noninteractive automation:

```bash
claude --strict-mcp-config --mcp-config ./.mcp.json -p "Inspect the schema"
```

## Troubleshooting

`connection_ambiguous` means multiple current-project or global profiles are
visible. Retry with the exact profile name or UUID. `connection_not_found` also
covers a profile assigned only to another project. `credential_failure` means
LazyDB could not resolve a password without interaction; Prompt profiles are
not usable by headless agents.

The MCP process loads profile metadata at startup. Restart the MCP session after
changing profiles or credentials in the LazyDB TUI.
