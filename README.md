<p align="center"><strong>LazyDB</strong> is a keyboard-first database workspace for the terminal.
<p align="center"> </p>
</br>
LazyDB lets you browse schemas, write and run SQL, inspect relations, manage
connection profiles, and expose project-scoped database access to coding agents
without leaving your terminal. It is written in Rust and supports PostgreSQL,
Oracle MySQL, SQL Server, and SQLite.

> **Project status:** LazyDB is currently in beta. The core database workspace,
> connection management, SQL execution, and coding-agent interfaces are usable,
> but LazyDB is not yet a production-ready replacement for DataGrip.

<table>
  <tr>
    <th>SQL Editor & Execution</th>
    <th>Table Data View</th>
  </tr>
  <tr>
    <td>
      <img alt="sql-query" src="https://github.com/user-attachments/assets/a37348f0-bc37-49ae-b164-034730a5217e" />
    </td>
    <td>
      <img alt="table-view-where-orderby" src="https://github.com/user-attachments/assets/69a60235-8a2f-49cb-8c4c-25392f33797c" />
    </td>
  </tr>
  <tr>
    <th>Dashboard</th>
    <th>Mouse Support</th>
  </tr>
  <tr>
    <td>
      <img alt="lazydb-dashboard" src="https://github.com/user-attachments/assets/439f3d1b-611d-42db-9bc9-039799f9e3d3" />
    </td>
    <td>
      <img alt="mouse-support" src="https://github.com/user-attachments/assets/bf39b262-b7b1-4f21-b1cc-fe8a58be7928" />
    </td>
  </tr>
  <tr>
    <th>MCP & Neovim Integrated</th>
    <th>Neovim LSP(Completion & Syntax Highlighting)</th>
  </tr>
  <tr>
    <td>
      <img alt="lazydb-mcp" src="https://github.com/user-attachments/assets/f4b03b84-0386-4090-8bc5-81178d29d360" />
    </td>
    <td>
      <img alt="lazydb-lsp" src="https://github.com/user-attachments/assets/63dee305-ec5a-4262-8366-2afdf52591d9" />
    </td>
  </tr>
</table>

---

## Quickstart

Press `F2` anywhere in the workspace to open the global Omni Bar for commands,
connections, consoles, tables, and navigation. See the [Omni Bar guide](docs/omni-bar.md).

### Installing and running LazyDB

Run the following on Mac or Linux to install LazyDB

```bash
curl -fsSL https://lazydb.yelog.org/install.sh | sh
```

Run the following on Windows to install LazyDB:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "irm https://lazydb.yelog.org/install.ps1.txt -ErrorAction Stop | iex"
```

After an interactive installation, the installer can open the LazyDB MCP
setup wizard. It supports user-level and project-level configuration and
shows the target file before writing it. To defer this step, run
`lazydb mcp setup` later, or use `--mcp-setup skip` with the Unix installer.

The Windows installer supports 64-bit Windows (MSVC), downloads the release
metadata and ZIP archive over HTTPS, verifies the SHA-256 checksum, and adds
`%LOCALAPPDATA%\LazyDB\bin` to the user `PATH`. No administrator privileges are
required. Open a new terminal after installation. To install the beta channel,
set `$env:LAZYDB_CHANNEL = "beta"` before running the command.

The command downloads and executes a script from the LazyDB site. The `.txt`
endpoint is generated from the same installer source, but served as text for
PowerShell compatibility. Review the [installer source](pages/install.ps1) if
needed, or download the Windows ZIP from the
[latest GitHub Release](https://github.com/yelog/lazydb/releases/latest).

<details>
<summary>Alternative: download the installer to a temporary file</summary>

For troubleshooting or environments where piping scripts to `Invoke-Expression`
is unsuitable, run the following in PowerShell. This also executes downloaded
code; saving it to a file does not authenticate the installer.

```powershell
& {
    $ErrorActionPreference = 'Stop'
    $installer = Join-Path ([IO.Path]::GetTempPath()) (
        [IO.Path]::GetRandomFileName() + '.ps1'
    )
    try {
        Invoke-WebRequest -Uri 'https://lazydb.yelog.org/install.ps1' `
            -UseBasicParsing -OutFile $installer
        if ((Get-Item -LiteralPath $installer).Length -eq 0) {
            throw 'Downloaded installer is empty.'
        }
        powershell.exe -NoProfile -ExecutionPolicy Bypass -File $installer
        if ($LASTEXITCODE -ne 0) {
            throw "LazyDB installer failed with exit code $LASTEXITCODE."
        }
    } finally {
        Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
    }
}
```

</details>

After installation, configure database access for Claude Code, Codex, or
OpenCode from the target project:

```bash
lazydb mcp setup
```

### Uninstalling LazyDB

Native installations can be inspected without changes and then removed with
the CLI:

```bash
lazydb uninstall --dry-run
lazydb uninstall
```

The default uninstall removes the LazyDB launcher, native releases, current
release link, installation metadata, and the PATH block managed by the LazyDB
installer. It preserves connection profiles, encrypted credential material,
settings, workspace files, SQL files, and unknown files. Use `--yes` for a
non-interactive uninstall. Use `--json` with `--dry-run` or `--yes` for a
machine-readable report.

`--purge` additionally removes the allowlisted LazyDB data files after profile
and credential checks. It may remove saved connection definitions, local
credential keys, settings, workspace state, and update-check state. Unknown
files and database files are preserved. System keyring entries are removed
only when they are referenced by valid LazyDB profiles and the native secret
store is available:

```bash
lazydb uninstall --purge --dry-run
lazydb uninstall --purge
```

Uninstall does not modify project MCP configuration. Remove the `lazydb` entry
from `.mcp.json`, `.codex/config.toml`, or `opencode.json[c]` in each project
before or after uninstall. Homebrew, Cargo, and system-package installations
must be removed through their original package manager. Windows uninstall is
not provided by the native POSIX command yet. If installation metadata is
missing or paths no longer match the recorded native installation, uninstall
stops without deleting anything and reports the paths that need manual review.

The setup command uses a project-scoped MCP configuration and denies database
writes by default. It does not install a coding agent or copy credentials.
Native script installers may offer an opt-in reminder on an interactive first
install. Use `--mcp-setup skip` or `LAZYDB_MCP_SETUP=skip` for unattended runs;
the prompt never changes configuration automatically because the installer does
not know which project should receive the MCP entry.

LazyDB can also be installed via Homebrew, Cargo, or by building from source. See

```bash
# install using Homebrew
brew install yelog/tap/lazydb

```

<details>
<summary>You can also go to the <a href="https://github.com/yelog/lazydb/releases/latest">latest GitHub Release</a> and download the appropriate binary for your platform.</summary>

Each GitHub Release contains many executables, but in practice, you likely want one of these:

- macOS
  - Apple Silicon/arm64: `lazydb_xxx_aarch64-apple-darwin.tar.xz`
  - x86_64 (older Mac hardware): `lazydb_xxx_x86_64-apple-darwin.tar.xz`
- Linux
  - x86_64: `lazydb_xxx_x86_64-unknown-linux-gnu.tar.xz`
  - arm64: `lazydb_xxx_aarch64-unknown-linux-gnu.tar.xz`

For example, on Apple silicon macOS with `v0.1.0-beta.2`:

```bash
tar -xJf lazydb_0.1.0-beta.2_aarch64-apple-darwin.tar.xz
mkdir -p "$HOME/.local/bin"
cp lazydb_0.1.0-beta.2_aarch64-apple-darwin/lazydb "$HOME/.local/bin/lazydb"
chmod +x "$HOME/.local/bin/lazydb"
lazydb version
```

Ensure `$HOME/.local/bin` is in `PATH`. To upgrade an offline installation,

</details>

## Features

- **Database Explorer:** Browse databases, schemas, tables, views, indexes,
  foreign keys, triggers, routines, and types where supported by the driver.
- **PostgreSQL catalog editor:** Capability-aware Explorer `a` creates supported
  children and `e` edits directly selected objects. The implemented scope is
  Schema, Table, Column, Index, Constraints, View, Materialized View, Sequence,
  Database, and Role. The Role node is not present in Explorer, so existing
  roles cannot currently be selected for editing.
- **Catalog reconciliation:** Explorer `r` refreshes only the selected target;
  PostgreSQL relation identities can be rebound after an external table rename,
  while SQL catalog changes trigger a conservative database-catalog sync.
- **SQL workspace:** Work with multiple console tabs, Vim-style Normal/Insert
  modes, search, formatting, syntax highlighting, and catalog-aware completion.
- **Safe execution:** Run the current statement or full buffer with scoped
  execution, immutable previews, risk confirmation, cancellation, and read-only
  safeguards.
- **Transactions:** Use per-console AUTO or MANUAL transactions on PostgreSQL,
  MySQL, SQL Server, and SQLite sessions, including rollback on cancellation and
  unknown outcome handling.
- **Relation workspaces:** Preview relation data with a bounded 500-row limit,
  switch between Data and DDL, and open DDL in a separate tab.
- **Connection profiles:** Create, test, save, edit, delete, disconnect, and
  switch PostgreSQL, MySQL, SQL Server, and SQLite profiles. Profiles can be
  saved, ad hoc, or scoped to the current project.
- **Credential protection:** Use local authenticated encryption by default, or
  macOS Login Keychain and Linux Secret Service when available.
- **Coding-agent access:** Use project-aware JSON CLI commands or a local stdio
  MCP server from Codex, OpenCode, or Claude Code.
- **Neovim integration:** Run the standalone `lazydb.nvim` floating-terminal
  plugin with one LazyDB process per Neovim tab.
- **Terminal-native UI:** Use keyboard navigation, mouse hit regions, truecolor
  themes, responsive 80-column focus mode, configurable motion feedback, and
  mouse drag selection in rendered text views. Explicit copy commands/buttons
  copy complete cell, row, SQL, or detail values where documented; drag alone
  never writes to the clipboard.
- **External themes:** Pass `--theme-file path/to/theme.json` to load and watch
  a `theme-file-v1` JSON theme. Invalid or unavailable files keep the built-in
  theme, and `--color never` always takes precedence.

See the [configuration guide](docs/configuration.md#external-theme-file) for
the complete theme-file protocol, limits, fallback behavior, and Neovim
plugin configuration.

## Supported Databases

| Database | Requirement | Catalog support |
| --- | --- | --- |
| PostgreSQL | 12 or newer | Databases, schemas, tables, columns, indexes, constraints, views, materialized views, sequences, functions, procedures, types; catalog editing also covers databases and roles; relation rename reconciliation uses readable `pg_class`/`pg_namespace` OIDs |
| Oracle MySQL | 8.0.13 or newer | Databases, tables, views, functions, procedures, triggers |
| SQL Server | SQL Server 2012 or newer | Databases, schemas, tables, views, functions, procedures, sequences, triggers, indexes, keys, foreign keys, and column metadata |
| SQLite | Native SQLite schema support | Tables, views, indexes, foreign keys, and triggers |

MariaDB is not part of the current MySQL catalog contract. See the complete
[database capability matrix](docs/database-capabilities.md) for metadata,
paging, relation DDL, and version details.

## Coding-Agent Access

LazyDB exposes the same native database adapters through a machine-readable JSON
CLI and a local stdio MCP server. Agent-visible profiles come from the current
Git project and global profiles. Profiles assigned only to other projects are
hidden.

Read-only agent workflow:

```bash
lazydb agent connections --project .
lazydb agent context --project .
lazydb agent schema-search users --project . --connection orders-dev
lazydb agent query --project . --connection orders-dev \
  --sql 'SELECT * FROM users LIMIT 20'
```

For SQL files, use an explicitly supplied project-relative path:

```bash
lazydb agent execute --project . --connection orders-dev \
  --file db/migrations/001.sql --write-policy non-production
```

The MCP server defaults to `--write-policy deny`. Database roles and grants
remain the final authorization boundary; client-side MCP approval does not
relax LazyDB or database permissions. See
[Coding-Agent Database Access](docs/coding-agent-access.md) for Codex, OpenCode,
Claude Code, permissions, and troubleshooting.

## Neovim Integration

[`yelog/lazydb.nvim`](https://github.com/yelog/lazydb.nvim) starts the LazyDB
executable in a floating terminal. Install the CLI first, then add the plugin
with `lazy.nvim`:

```lua
return {
  {
    "yelog/lazydb.nvim",
    cmd = { "LazyDB", "LazyDBToggle", "LazyDBHide", "LazyDBStop", "LazyDBRestart" },
    opts = {
      executable = "lazydb",
      window = { width = 0.92, height = 0.90, border = "rounded" },
    },
  },
}
```

Use `:checkhealth lazydb` to verify the executable and CLI API. See the plugin
repository for native package installation and the complete command reference.

The standalone LSP supports profile-backed catalog completion for Neovim. It
replaces only the current identifier segment, so accepting `sys_user` inserts
`sys_user` rather than an automatically generated full path. After typing a
dot, completion navigates the next catalog level: `database.` lists schemas,
`database.schema.` lists tables/views, and `schema.` resolves inside the active
database. Candidate details show the database/schema source for ambiguous
names; visible aliases such as `u.` load their relation columns on demand.

The exact path syntax follows the selected profile dialect. SQL Server supports
three-part names, MySQL folds its database/schema mirror, SQLite uses attached
database names such as `main`, and PostgreSQL does not present ordinary
cross-database paths. Profile `catalog_scope` and database permissions remain
the visibility boundary.

The footer and help view show contextual controls for the active context and mode. See the
complete [keyboard reference](docs/keybindings.md) for the operational contract,
including the distinction between application `Space` commands in Explorer/Results
and editor `Space`/`\\` commands in SQL Editor Normal/Visual mode, as well as profile,
relation, result-grid, search, and confirmation controls.

## Configuration

Connection profiles, the local credential key, and workspace state are stored in
`~/lazydb/` on macOS and Linux by default. Existing installations continue to
use their current root during upgrades. Set `LAZYDB_CONFIG_HOME` to
use another directory for these files. For example, to keep using the previous
macOS location:

```bash
export LAZYDB_CONFIG_HOME=$HOME/lazydb
```

Existing users are not moved automatically. After reviewing the plan, move a
complete Unix application root with `lazydb migrate-home --to
"$HOME/lazydb" --dry-run`, followed by the same command with `--yes`.

This changes the configuration root; it does not automatically copy connection
profiles from the default directory. If profiles use local encrypted
credentials, keep the matching `credential.key` with `connections.toml` when
migrating them.

Windows continues to use `%APPDATA%\\lazydb\\`. The directory contains
`settings.toml`, `connections.toml`, `credential.key`, `workspace.toml`, and the `sql/` directory
for persisted console text. Use `--config PATH` to select a different profile
file for the current run; this does not relocate the key or workspace files.
The complete built-in application defaults are in
[`config/default.toml`](config/default.toml). This file is embedded into the
binary and is the authoritative source of defaults; a user `settings.toml` only
needs to contain overrides. Explicit command-line options take precedence.

Clipboard writes use the system clipboard by default. Set
`[terminal.clipboard].backend = "osc52"` in `settings.toml` to send OSC 52
sequences instead, or use `"off"` to disable clipboard writes. `max_bytes`
limits the encoded OSC 52 payload and oversized values are rejected rather than
truncated. `Ctrl-Shift-s` releases mouse capture for terminal-native
selection; `Esc` restores it. `Ctrl-c` remains the quit key outside
context-specific cancellation. Terminal, SSH, and tmux clipboard behavior is
environment-dependent and requires manual QA in the actual terminal path.
Common options include:

```bash
lazydb --profile NAME
lazydb --read-only --profile NAME
lazydb --icons ascii
lazydb --motion reduced
lazydb --mouse off
```

Use `--icons unicode` or `--icons ascii` for terminals without Nerd Font
support. Use `--motion reduced` or `--motion off` to reduce or disable loading
animation. These display options apply to the current process only.

Read the complete [Configuration Guide](docs/configuration.md) for every command-line
option, configuration and workspace file, connection-profile field, default value,
project scope, password provider, TLS mode, and read-only behavior.

## Current Limitations

The following capabilities are not available yet:

- Persistent console recovery and console renaming
- Staged grid editing, optimistic conflict detection, insert, and delete
- Query plans
- SSH tunneling
- Import and export

SQL Server currently uses SQL username/password authentication over an explicit
TCP host and port. Windows Integrated Authentication, Kerberos, Entra
authentication, Named Instances and SQL Browser discovery are deferred. SQL
Server Dashboard metrics, process metrics, and additional specialized type
handling are also deferred. Cancelling SQL Server work closes the active session;
if that session has an open transaction, SQL Server rolls it back.

The project is evolving quickly during beta. See the issue tracker and release
notes for current priorities and version-specific changes.

## Documentation

| Topic | Document |
| --- | --- |
| Built-in default configuration | [`config/default.toml`](config/default.toml) |
| Configuration and connection profiles | [`docs/configuration.md`](docs/configuration.md) |
| Database capability matrix | [`docs/database-capabilities.md`](docs/database-capabilities.md) |
| Keyboard reference | [`docs/keybindings.md`](docs/keybindings.md) |
| Coding-agent and MCP access | [`docs/coding-agent-access.md`](docs/coding-agent-access.md) |
| Product design | [`docs/plans/2026-08-24-lazydb-design.md`](docs/plans/2026-08-24-lazydb-design.md) |

## Development

Run the standard local checks before opening a pull request:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

The test suite covers database adapters, catalogs, connections, workspace
behavior, CLI behavior, and coding-agent access boundaries.

## Security

- Passwords are never stored in plain text connection URLs.
- Saved credentials use local authenticated encryption or a native OS secret
  store when configured.
- Read-only mode is enforced by the adapter where supported; use a database role
  with read-only grants for the actual authorization boundary.
- MCP write access is denied by default.
- Database and user-provided error text is sanitized before terminal display.

For the detailed security model, see [Configuration](docs/configuration.md) and
[Coding-Agent Database Access](docs/coding-agent-access.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
