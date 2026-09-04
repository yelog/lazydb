# LazyDB

**A keyboard-first database workspace for the terminal.**

LazyDB lets you browse schemas, write and run SQL, inspect relations, manage
connection profiles, and expose project-scoped database access to coding agents
without leaving your terminal. It is written in Rust and supports PostgreSQL,
Oracle MySQL, SQL Server, and SQLite.

> **Project status:** LazyDB is currently in beta. The core database workspace,
> connection management, SQL execution, and coding-agent interfaces are usable,
> but LazyDB is not yet a production-ready replacement for DataGrip.

## Features

- **Database Explorer:** Browse databases, schemas, tables, views, indexes,
  foreign keys, triggers, routines, and types where supported by the driver.
- **PostgreSQL catalog editor:** Capability-aware Explorer `a` creates supported
  children and `e` edits directly selected objects. The implemented scope is
  Schema, Table, Column, Index, Constraints, View, Materialized View, Sequence,
  Database, and Role. The Role node is not present in Explorer, so existing
  roles cannot currently be selected for editing.
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
  themes, responsive 80-column focus mode, and configurable motion feedback.

## Supported Databases

| Database | Requirement | Catalog support |
| --- | --- | --- |
| PostgreSQL | 12 or newer | Databases, schemas, tables, columns, indexes, constraints, views, materialized views, sequences, functions, procedures, types; catalog editing also covers databases and roles |
| Oracle MySQL | 8.0.13 or newer | Databases, tables, views, functions, procedures, triggers |
| SQL Server | SQL Server 2012 or newer | Databases, schemas, tables, views, functions, procedures, sequences, triggers, indexes, keys, foreign keys, and column metadata |
| SQLite | Native SQLite schema support | Tables, views, indexes, foreign keys, and triggers |

MariaDB is not part of the current MySQL catalog contract. See the complete
[database capability matrix](docs/database-capabilities.md) for metadata,
paging, relation DDL, and version details.

## Installation

### Homebrew

On supported macOS systems, install the latest stable release from the external
tap:

```bash
brew install yelog/tap/lazydb
```

Homebrew owns this installation. Upgrade it with `brew update` followed by
`brew upgrade yelog/tap/lazydb`, not with `lazydb update`.

### Pages installer

The stable Pages installer is the canonical macOS and Linux installation path.
It verifies a channel manifest and the matching archive SHA-256 digest, then
installs to `~/.local/bin` by default. It never uses `sudo`:

```bash
curl -fsSL https://lazydb.yelog.org/install.sh | sh
```

The beta installer is clearly separate and never changes the stable channel:

```bash
curl -fsSL https://lazydb.yelog.org/install-beta.sh | sh
```

Beta is for testing prereleases. It does not update Homebrew, the stable
installer, or the `latest` release path. To inspect either installer before
running it, download it first:

```bash
curl -fsSL \
  https://lazydb.yelog.org/install.sh \
  -o lazydb-installer.sh
less lazydb-installer.sh
sh lazydb-installer.sh --install-dir "$HOME/.local/bin"
```

The installer supports `--channel stable|beta`, `--version VERSION`, and
`--install-dir PATH`. The stable and beta Pages entrypoints lock their own
channel, so use the matching entrypoint when selecting a channel explicitly.

### Offline installation from GitHub Releases

For a machine without internet access, download the matching archive from
[GitHub Releases](https://github.com/yelog/lazydb/releases) on a connected
machine, then transfer it to the offline machine. Each archive contains a
standalone `lazydb` executable; it is not an `apt`, DNF, or Pacman package.

Choose the archive that matches the offline machine:

| Platform | Release target |
| --- | --- |
| macOS on Apple silicon | `aarch64-apple-darwin` |
| macOS on Intel | `x86_64-apple-darwin` |
| Linux on ARM64 | `aarch64-unknown-linux-gnu` |
| Linux on x86-64 | `x86_64-unknown-linux-gnu` |

After transferring the archive to the offline machine, extract it, create a
user-local binary directory, copy the executable, and verify the installation.
For example, on Apple silicon macOS with `v0.1.0-beta.2`:

```bash
tar -xJf lazydb_0.1.0-beta.2_aarch64-apple-darwin.tar.xz
mkdir -p "$HOME/.local/bin"
cp lazydb_0.1.0-beta.2_aarch64-apple-darwin/lazydb "$HOME/.local/bin/lazydb"
chmod +x "$HOME/.local/bin/lazydb"
lazydb version
```

Ensure `$HOME/.local/bin` is in `PATH`. To upgrade an offline installation,
repeat the download, transfer, extraction, and replacement steps with the newer
release. Use the [Pages installer](#pages-installer) instead when the target
machine has internet access and should receive automatic channel updates.

### Cargo package

For a source-based Cargo installation:

```bash
cargo install lazydb
```

Cargo owns this installation. Upgrade it with `cargo install lazydb`; the
application reports that manager action rather than replacing the Cargo binary.

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
lazydb version
lazydb doctor
```

Check for an available update without changing files:

```bash
lazydb update --check
```

Apply an update for a native Pages installation:

```bash
lazydb update
```

`lazydb update` checks the installed manager and selected channel. Native Pages
installations download and verify the channel manifest and apply a newer
release atomically. `lazydb update --check` performs the same check but never
applies it. Use `--channel beta` or `--channel stable` to select a channel for
the operation; a successful native update records that channel. Homebrew,
Debian, RPM, Arch, and Cargo installations remain owned by their managers and
receive an explicit manager command instead. npm-managed installations are
detected and protected, but official npm distribution is currently unavailable;
use the Pages installer or Homebrew instead.

Inside the TUI, the header shows the running version. Press `F9` or click the
version/update badge to open the Update Center. Startup checks run asynchronously
after the first frame and are cached for 24 hours by default. A native update can
be installed without interrupting the current session; the current process keeps
running until `Restart now` is selected. `Later` leaves the session untouched.
Package-manager and source installations are never overwritten and instead show
the appropriate upgrade guidance.

Start LazyDB:

```bash
lazydb
```

On first launch, the Profile Manager opens automatically. Select PostgreSQL,
MySQL, SQL Server, or SQLite; enter the connection details; test the connection;
then save and connect. Saved profiles remain available the next time LazyDB
starts, and passwords can use local encrypted storage or the native
operating-system secret store when available.

### Explorer Connection Groups

Saved connections can belong to at most one custom group. In Explorer, a group
is projected independently in the primary and `others` regions when it has
members in both; empty groups remain available in the group picker but are not
shown as tree rows. Use `a` to create a group, `e` to rename a selected group,
`d` to delete it, and `g` on a saved profile to change its membership. Deleting
a group ungroups its connections without deleting them. `J` and `K` reorder a
connection only among visible siblings in the same region and group.

The V6 `connections.toml` format stores group metadata and profile order. It
does not store plaintext credentials:

```toml
version = 6

[[groups]]
id = "11111111-1111-1111-1111-111111111111"
name = "Production"

[[profiles]]
id = "22222222-2222-2222-2222-222222222222"
name = "Billing"
group_id = "11111111-1111-1111-1111-111111111111"
# existing connection fields follow
```

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

The footer and help view show contextual controls for the active context and mode. See the
complete [keyboard reference](docs/keybindings.md) for the operational contract,
including the distinction between application `Space` commands in Explorer/Results
and editor `Space`/`\\` commands in SQL Editor Normal/Visual mode, as well as profile,
relation, result-grid, search, and confirmation controls.

## Configuration

Connection profiles, the local credential key, and workspace state are stored in
`~/.config/lazydb/` on macOS and Linux by default. Set `LAZYDB_CONFIG_HOME` to
use another directory for these files. For example, to keep using the previous
macOS location:

```bash
export LAZYDB_CONFIG_HOME=$HOME/lazydb
```

Windows continues to use `%APPDATA%\\lazydb\\`. The directory contains
`settings.toml`, `connections.toml`, `credential.key`, `workspace.toml`, and the `sql/` directory
for persisted console text. Use `--config PATH` to select a different profile
file for the current run; this does not relocate the key or workspace files.
The complete built-in application defaults are in
[`config/default.toml`](config/default.toml). This file is embedded into the
binary and is the authoritative source of defaults; a user `settings.toml` only
needs to contain overrides. Explicit command-line options take precedence.
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
