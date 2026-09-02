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

Approving an MCP tool call only permits OpenCode to send the request. It does
not override the LazyDB process policy. If the MCP command contains
`--write-policy deny`, every `execute_change` and `execute_file` call is
rejected after client approval and before database connection.

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

For an explicitly writable development or staging session, use this variant
instead of the secure read-only example above:

```jsonc
{
  "mcp": {
    "lazydb": {
      "type": "local",
      "command": [
        "lazydb", "mcp", "serve", "--project", ".",
        "--write-policy", "non-production"
      ],
      "cwd": "."
    }
  },
  "permission": {
    "lazydb_execute_change": "ask",
    "lazydb_execute_file": "ask"
  }
}
```

Keep both write tools set to `ask`; client approval is an additional
confirmation layer and cannot override a read-only profile, production
restrictions, or database grants. Use `all` for production only as part of an
approved workflow.

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

`get_context` reports the MCP process policy and the selected connection's
effective write capability:

```json
{
  "server_write_policy": "deny",
  "write_capability": {
    "allowed": false,
    "denial_reason": "server_policy",
    "message": "the MCP server write policy is deny; restart it with --write-policy non-production for writable development or staging connections"
  }
}
```

| `denial_reason` | Cause | Correct action |
| --- | --- | --- |
| `server_policy` | MCP started with `deny` | Change to `non-production` for development/staging and restart OpenCode |
| `profile_read_only` | LazyDB profile has `read_only = true` | Keep it read-only or deliberately edit the profile; MCP approval cannot override it |
| `production_policy` | Production profile under `non-production` | Use an approved production workflow and explicitly start with `all` only when required |

`connection_ambiguous` means multiple current-project or global profiles are
visible. Retry with the exact profile name or UUID. `connection_not_found` also
covers a profile assigned only to another project. `credential_failure` means
LazyDB could not resolve a password without interaction; Prompt profiles are
not usable by headless agents.

The MCP process loads profile metadata at startup. Restart the MCP session after
changing profiles, credentials, or MCP command arguments. OpenCode also loads
its configuration at startup, so fully quit and restart OpenCode after changing
the MCP configuration.
