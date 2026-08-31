# LazyDB

**A keyboard-first database workspace for the terminal.**

LazyDB lets you browse schemas, write and run SQL, inspect relations, manage
connection profiles, and expose project-scoped database access to coding agents
without leaving your terminal. It is written in Rust and supports PostgreSQL,
Oracle MySQL, and SQLite.

> **Project status:** LazyDB is currently in beta. The core database workspace,
> connection management, SQL execution, and coding-agent interfaces are usable,
> but LazyDB is not yet a production-ready replacement for DataGrip.

## Features

- **Database Explorer:** Browse databases, schemas, tables, views, indexes,
  foreign keys, triggers, routines, and types where supported by the driver.
- **SQL workspace:** Work with multiple console tabs, Vim-style Normal/Insert
  modes, search, formatting, syntax highlighting, and catalog-aware completion.
- **Safe execution:** Run the current statement or full buffer with scoped
  execution, immutable previews, risk confirmation, cancellation, and read-only
  safeguards.
- **Transactions:** Use per-console AUTO or MANUAL transactions on PostgreSQL,
  MySQL, and SQLite sessions, including rollback on cancellation and unknown
  outcome handling.
- **Relation workspaces:** Preview relation data with a bounded 500-row limit,
  switch between Data and DDL, and open DDL in a separate tab.
- **Connection profiles:** Create, test, save, edit, delete, disconnect, and
  switch PostgreSQL, MySQL, and SQLite profiles. Profiles can be saved, ad hoc,
  or scoped to the current project.
- **Credential protection:** Use local authenticated encryption by default, or
  macOS Login Keychain and Linux Secret Service when available.
- **Coding-agent access:** Use project-aware JSON CLI commands or a local stdio
  MCP server from Codex, OpenCode, or Claude Code.
- **Neovim integration:** Run the standalone `lazydb.nvim` floating-terminal
  plugin with one LazyDB process per Neovim tab.
- **Terminal-native UI:** Use keyboard navigation, mouse hit regions, truecolor
  themes, responsive 80-column focus mode, and configurable motion feedback.

## Supported Databases

| Database | Requirement | Catalog support |
| --- | --- | --- |
| PostgreSQL | 12 or newer | Databases, schemas, tables, views, materialized views, sequences, functions, procedures, types |
| Oracle MySQL | 8.0.13 or newer | Databases, tables, views, functions, procedures, triggers |
| SQLite | Native SQLite schema support | Tables, views, indexes, foreign keys, and triggers |

MariaDB is not part of the current MySQL catalog contract. See the complete
[database capability matrix](docs/database-capabilities.md) for metadata,
paging, relation DDL, and version details.

## Installation

### Homebrew

On supported macOS systems:

```bash
brew install yelog/tap/lazydb
```

### Installer

On macOS and Linux, the stable installer verifies the release archive against
its SHA-256 manifest and installs to `~/.local/bin` by default:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/yelog/lazydb/releases/latest/download/lazydb-installer.sh | sh
```

To inspect the installer before running it, download it first:

```bash
curl --proto '=https' --tlsv1.2 -fL \
  https://github.com/yelog/lazydb/releases/latest/download/lazydb-installer.sh \
  -o lazydb-installer.sh
less lazydb-installer.sh
sh lazydb-installer.sh --install-dir "$HOME/.local/bin"
```

### Release binaries and packages

Download stable or beta archives, checksums, and native Linux packages from
[GitHub Releases](https://github.com/yelog/lazydb/releases). Beta releases are
published as prereleases and do not update Homebrew or the stable
`releases/latest` installer. Verify an archive with its accompanying
`SHA256SUMS` file before extracting it.

### Build from source

Requirements:

- Rust 1.94 or newer
- macOS or Linux
- A UTF-8 terminal; truecolor is recommended
- Nerd Fonts 3.x are recommended for branded database icons

```bash
cargo build --release
```

Unicode and ASCII icon fallbacks are available when Nerd Font glyphs are not
installed.

## Quick Start

Verify a binary installation:

```bash
lazydb version --json
lazydb doctor --json
```

Start an in-memory SQLite workspace:

```bash
lazydb --url sqlite::memory:
```

Open a SQLite file:

```bash
lazydb --url sqlite:///tmp/lazydb-demo.db
```

Connect to PostgreSQL or MySQL without putting a password in process arguments:

```bash
LAZYDB_PASSWORD='session-only-password' \
  lazydb --url 'postgresql://alice@localhost:5432/app?sslmode=require'

LAZYDB_PASSWORD='session-only-password' \
  lazydb --url 'mysql://alice@localhost:3306/app?sslMode=REQUIRED'
```

Do not include passwords in `--url`; command-line arguments may be visible to
other local processes. `LAZYDB_PASSWORD` is read only at startup and is bound to
the selected profile or ad hoc connection. It is never written to disk.

You can also start LazyDB without arguments and create a connection in the
Profile Manager. `--url` creates a session-only profile; `--profile NAME`
selects a saved profile. If both are supplied, `--url` takes precedence.

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

## Essential Keys

| Action | Key |
| --- | --- |
| Help | `F1` or `Space ?` |
| Focus Explorer | `Space c` |
| New SQL console | `Space n` |
| Catalog search | `/` |
| Move through a Vim-style workspace | `h/j/k/l` |
| Run current statement | `F5` |
| Run full console buffer | `Shift-F5` |
| Refresh Explorer | `r` |
| Quit | `Q` |

The footer and help view show the active context. See the complete
[keybindings reference](docs/keybindings.md), including profile, relation, and
result-grid actions.

## Configuration

Connection profiles are stored in the platform configuration directory. Use
`--config PATH` to select a different profile file for the current run. Common
options include:

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

Read [Configuration](docs/configuration.md) for profile storage, project scope,
ad hoc URLs, password providers, TLS, and read-only behavior.

## Current Limitations

The following capabilities are not available yet:

- Persistent console recovery and console renaming
- Staged grid editing, optimistic conflict detection, insert, and delete
- Query plans
- SSH tunneling
- Import and export

The project is evolving quickly during beta. See the issue tracker and release
notes for current priorities and version-specific changes.

## Documentation

| Topic | Document |
| --- | --- |
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
