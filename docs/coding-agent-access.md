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

Create or merge the project `opencode.json` (OpenCode V2 native format):

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "lazydb": {
        "type": "local",
        "command": ["lazydb", "mcp", "serve", "--project", ".", "--write-policy", "deny"],
        "cwd": ".",
        "codemode": false
      }
    }
  },
  "permissions": [
    { "action": "lazydb_get_context", "resource": "*", "effect": "allow" },
    { "action": "lazydb_list_connections", "resource": "*", "effect": "allow" },
    { "action": "lazydb_search_schema", "resource": "*", "effect": "allow" },
    { "action": "lazydb_describe_object", "resource": "*", "effect": "allow" },
    { "action": "lazydb_query", "resource": "*", "effect": "allow" },
    { "action": "lazydb_execute_change", "resource": "*", "effect": "ask" },
    { "action": "lazydb_execute_file", "resource": "*", "effect": "ask" }
  ]
}
```

OpenCode V1 uses the same MCP server command under `mcp.lazydb`. LazyDB setup
reads both layouts and preserves the existing layout. Use
`--opencode-format v1` or `--opencode-format v2` to select the format for a new
entry; `auto` preserves an existing entry and can detect an explicit
`opencode1`/`opencode2` entrypoint.

Use the diagnostic command before starting OpenCode:

```bash
lazydb mcp doctor --client opencode --opencode-bin opencode2 --probe --json
```

`--probe` verifies the local process, MCP initialization, and `tools/list`; it
does not execute a database query. If it reports a profile parsing error,
rebuild LazyDB and point the server command at that verified binary. The
installed binary and the profiles file must support the same database drivers.

For an explicitly writable development or staging session, use this variant
instead of the secure read-only example above:

```jsonc
{
  "mcp": {
    "servers": {
      "lazydb": {
      "type": "local",
      "command": [
        "lazydb", "mcp", "serve", "--project", ".",
        "--write-policy", "non-production"
      ],
      "cwd": ".",
      "codemode": false
      }
    }
  },
  "permissions": [
    { "action": "lazydb_execute_change", "resource": "*", "effect": "ask" },
    { "action": "lazydb_execute_file", "resource": "*", "effect": "ask" }
  ]
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

## Automated Setup

The supported setup entry point is:

```bash
lazydb mcp setup
```

The Unix and Windows installers offer the same optional MCP onboarding. On a
first interactive installation, answer `y` to enter the setup wizard. The
wizard lets you choose a user-level or project-level configuration and shows
the exact file before writing it. Answer `n` or use `--mcp-setup skip` to defer
the setup; retry later with `lazydb mcp setup` or select a project explicitly:

```sh
curl -fsSL https://lazydb.yelog.org/install.sh | sh -s -- --mcp-setup ask
curl -fsSL https://lazydb.yelog.org/install.sh | sh -s -- --mcp-setup skip
lazydb mcp setup --project "/path/to/project"
```

Upgrades and non-interactive installs do not block waiting for MCP input. A
non-interactive install prints the same manual retry command instead.

It discovers existing Claude Code, Codex and OpenCode configuration and lets you
choose where to register LazyDB. Existing LazyDB entries are recommended first,
followed by existing user configuration. `--scope user` makes the server available
across projects; `--scope project` selects project configuration. Claude Code also
supports `--scope local` (private to the current project, stored in `.claude.json`).
Scripts that omit `--scope` retain the project default. All generated entries use
`--write-policy deny`. Use `--dry-run` to preview changes and `lazydb mcp doctor`
to inspect configuration without connecting to a database.

Installers and Homebrew print this command after installation but do not modify
agent configuration automatically. This keeps package installation safe for
unattended environments and leaves the project choice to the user.

The project directory defaults to the current directory; use `--project` to select
another directory. A project-scoped client configuration does not hide LazyDB
global profiles; the same profile visibility and selection rules above still
apply.

Setup preserves JSONC/TOML comments and unrelated fields when adding a server.
Plans distinguish `create`, `add`, `unchanged`, `conflict`, and `invalid`. Running
setup again leaves matching entries untouched, including custom options. A
different existing LazyDB entry is never overwritten by `--yes`; review it
manually. Disabled entries remain disabled and are reported as such.

Discovery includes OpenCode's XDG user directory, `OPENCODE_CONFIG`, project and
`.opencode` files, and `OPENCODE_CONFIG_DIR`; Codex's `CODEX_HOME` (default
`~/.codex`) and repository config layers; and Claude Code's user/local
`~/.claude.json` plus project `.mcp.json`. A custom `CLAUDE_CONFIG_DIR` uses its
`.claude.json`. Multiple existing targets in an explicitly selected scope require
`--client-config <path>` (one client only). This option is also available to doctor.
An explicit path does not cause a client to load that file automatically.

User registration does not pin the project used during setup: `--project .` is
resolved when the MCP process starts. OpenCode uses workspace-relative `cwd`;
Codex uses the server's launch directory. Generated Claude Code entries omit
`--project` so LazyDB can use the client's `CLAUDE_PROJECT_DIR`, falling back to
the working directory on older clients. Explicit `--project` takes precedence. LazyDB
`--config` paths supplied to setup are resolved to absolute paths. Codex project
configuration requires project trust; generated Codex entries are optional and
do not make Codex startup fail if LazyDB cannot initialize.

Useful non-interactive forms are:

```bash
lazydb mcp setup --client claude-code --client codex --project . --dry-run --json
lazydb mcp setup --client opencode --project . --yes --json
lazydb mcp setup --client opencode --scope user --dry-run --json
lazydb mcp setup --client codex --scope user --yes
lazydb mcp setup --client claude-code --scope local --project . --yes
lazydb mcp setup --client opencode --scope user --client-config ~/.config/opencode/opencode.jsonc --yes
lazydb mcp doctor --project . --json
```

The current `doctor --probe` flag reports that protocol probing is not yet
implemented and does not start configured client commands. A successful static
diagnosis confirms parsed server fields and deny policy, not client trust,
process startup, or database connectivity. Doctor lists discovered LazyDB sources
in file precedence order and reports duplicate definitions and disabled entries.
Runtime CLI overrides, inline OpenCode configuration, remote/managed settings and
client-specific trust decisions are not fully resolved by static inspection.
