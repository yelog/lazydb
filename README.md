# LazyDB

LazyDB is a keyboard-first, mouse-capable terminal database IDE written in Rust.
It aims to bring the core JetBrains Database Tools workflow to a fast,
standalone TUI with a distinctive deep-space visual system and a thin Neovim
integration.

> Project status: M1 editor and transaction foundation. The current build is
> useful for connection, catalog, scoped SQL editing/execution, and transaction
> exploration, but it is not yet a production-ready replacement for DataGrip.

## What Works

- PostgreSQL, MySQL, and SQLite connection and server probing. PostgreSQL 12+
  and Oracle MySQL 8.0.13+ are required for their catalog implementations.
- PostgreSQL/MySQL/SQLite URL import plus JDBC PostgreSQL/MySQL import.
- SQLite tables, views, columns, indexes, foreign keys, triggers, and DDL.
- PostgreSQL/MySQL databases, schemas, tables, views, columns, indexes, and
  foreign keys.
- Multiple SQL console tabs with a compact Vim-style Normal/Insert editor.
- Unicode-safe Vim modes, search, Ex commands, substitution, formatting,
  highlighting, and catalog-backed completion.
- Scoped execution with immutable previews, risk confirmation, and read-only
  adapter enforcement.
- Per-console AUTO/MANUAL transactions on pinned PostgreSQL, MySQL, and SQLite
  sessions, including cancellation rollback and unknown-outcome handling.
- Multi-statement execution, typed result decoding, Output history, and
  generation-safe asynchronous updates.
- Runtime profile manager for creating, testing, saving, editing, deleting, and
  switching PostgreSQL, MySQL, and SQLite connections.
- Password storage with Local Encrypted as the cross-platform default, plus
  detected macOS Login Keychain or Linux Secret Service support.
- UUID-owned Explorer roots for saved and session-only profiles, lazy catalog
  loading, server-backed `/` search across unloaded objects and relation children,
  object tree refresh, relation Data/DDL tabs, table/view preview with an
  adapter-owned 500-row limit, complete adapter-owned DDL, and DDL in a new tab.
- Responsive 80-column focus mode, standard split layout, truecolor theme,
  contextual help, bounded TachyonFX transitions, and mouse hit regions.
- A thin `lazydb.nvim` floating-terminal plugin with one process per Neovim tab.
- Stable machine-readable `version`, `capabilities`, and `doctor` commands.
- Configurable terminal motion feedback with `--motion full`, `--motion reduced`,
  and `--motion off`.
- SQL completion includes database/schema/relation paths, relation-aware columns,
  native column types, and catalog icons; the statement under the cursor is
  underlined when it is the current execution scope.

## Deliberately Deferred

The following items are in M1/M2 and have no fake controls in the current UI:

- Persistent console recovery and renaming.
- Where/Order By controls and selectable paging sizes.
- Staged grid editing, optimistic conflict detection, insert, and delete.
- Plans, SSH, import, and export.

See [the product design](docs/plans/2026-08-24-lazydb-design.md) and
[M0 implementation plan](docs/plans/2026-08-24-lazydb-m0-implementation.md).

## Build

Requirements:

- Rust 1.94 or newer.
- macOS or Linux.
- A UTF-8 terminal. Truecolor is recommended.
- Nerd Fonts 3.x (or a compatible Symbols Nerd Font fallback) is recommended
  for branded database and catalog icons. Unicode and ASCII fallbacks are
  available when Nerd Font glyphs are not installed.

```bash
cargo build --release
```

## Install

Stable releases are intended to be available from the project Homebrew tap on
supported macOS systems:

```bash
brew install yelog/tap/lazydb
```

The current `v0.1.0-beta.1` prerelease is distributed through GitHub Release
assets only. It does not update the stable Homebrew Formula.

On macOS and Linux, the stable release can also be installed without a package
manager. The installer verifies the downloaded archive against the release
SHA-256 manifest and installs to `~/.local/bin` by default:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/yelog/lazydb/releases/latest/download/lazydb-installer.sh | sh
```

To inspect the script before running it, download it first and invoke it from
the local file:

```bash
curl --proto '=https' --tlsv1.2 -fL \
  https://github.com/yelog/lazydb/releases/latest/download/lazydb-installer.sh \
  -o lazydb-installer.sh
less lazydb-installer.sh
sh lazydb-installer.sh --install-dir "$HOME/.local/bin"
```

Stable Linux releases also include native packages. These are release assets,
not repositories, so use the local-file or direct-URL forms below:

```bash
sudo apt install ./lazydb_VERSION_ARCH.deb
sudo dnf install https://github.com/yelog/lazydb/releases/download/TAG/lazydb-VERSION-ARCH.rpm
sudo pacman -U https://github.com/yelog/lazydb/releases/download/TAG/lazydb-VERSION-ARCH.pkg.tar.zst
```

Beta versions are published as GitHub prereleases with binary archives and
checksums only. Open the desired prerelease and download the archive matching
your operating system and architecture. Verify it with the accompanying
`SHA256SUMS` before extracting it. Beta versions do not update Homebrew or the
stable `releases/latest` installer.

Run an in-memory SQLite workspace:

```bash
cargo run -- --url sqlite::memory:
```

Use `--motion reduced` for low-frequency loading animation or `--motion off` for
static loading feedback. The default is `full`; this is a session-only option.

Run against a SQLite file:

```bash
cargo run -- --url sqlite:///tmp/lazydb-demo.db
```

Connect to PostgreSQL or MySQL without putting a password in process arguments:

```bash
LAZYDB_PASSWORD='session-only-password' \
  cargo run -- --url 'postgresql://alice@localhost:5432/app?sslmode=require'

LAZYDB_PASSWORD='session-only-password' \
  cargo run -- --url 'mysql://alice@localhost:3306/app?sslMode=REQUIRED'
```

Do not include passwords in `--url`; command-line arguments may be visible to
other local processes. `LAZYDB_PASSWORD` is read only at startup and is bound to
the selected persisted profile (or the ad-hoc `--url` connection). It is never
written to disk or reused for another profile.

On first launch with no saved profiles, LazyDB opens a new Profile Manager form
and the Explorer shows a `No profiles` row. This row starts a new draft; it is
not a profile-list popup. The Explorer has one UUID-targeted root per saved
profile and per current-process session profile. Roots show `SAVED` or `SESSION`
and `OFFLINE`, `LINKING`, `ONLINE`, `SYNCING`, or `FAILED`; catalog loading,
stale data, permission failures, and retries are shown below the owning root.

### Per-Connection Workspaces

Each connection profile has its own workspace of SQL and relation tabs. Switching
profiles changes the visible workspace only after the new connection succeeds. If
the connection attempt fails, the previous connection and workspace remain active.
Disconnecting a profile hides its workspace but does not delete it. Relation tabs
return as lazy shells when their workspace is shown again; their result data is
not persisted across an application restart. Deleting a profile also deletes its
workspace.

Press `Space c` from any normal-mode workspace to focus the connection Explorer. Use
`Test Connection` to validate a draft without persistence or changing the active
connection. A successful test also discovers databases and schemas for the
hierarchical scope picker. The picker supports `All` or `Selected` databases
and schema selection. MySQL mirrors each database as its schema, so its schema
rows are informational and not independently selectable. PostgreSQL exposes an
optional default schema; while visibility is not customized, it also limits the
Explorer to that database and schema. The URL field accepts native and JDBC
forms, fills the profile fields, and is regenerated when those fields change.
Any URL password is moved into the secret-backed Password field and removed from
the displayed URL. `Save` persists metadata, and `Save & Connect` persists and
  activates it. `Password Storage` defaults to `LOCAL ENCRYPTED`, which stores
  authenticated ciphertext in the profile file and a separate device-local key.
  When a supported native store is detected, the Profile Manager also offers
  `MACOS LOGIN KEYCHAIN` or `SECRET SERVICE`; unavailable providers are hidden
  for new connections. If a selected System store cannot
  save the password, LazyDB falls back to Local Encrypted storage and reports
  the actual storage mode.

## CLI Contract

```text
lazydb [--config PATH] [--profile NAME] [--read-only]
       [--mouse auto|on|off] [--color auto|always|never]
       [--icons nerd-font|unicode|ascii]
       [--confirm-execution risky|always]

lazydb version --json
lazydb capabilities --json
lazydb doctor --json [--profile NAME]
```

`--icons` applies to the current process only. `nerd-font` is the default and
uses recognizable database brand glyphs; `unicode` uses standard Unicode
symbols; `ascii` is the safest choice for minimal or remote terminals. If
Nerd Font icons appear as boxes or misalign, select a fallback mode.

`--url` creates an ad-hoc connection and never writes a profile. `--profile`
selects a saved profile by name; if both are supplied, `--url` wins.

## Essential Keys

| Context | Keys |
| --- | --- |
| Global | `F1` help, `Ctrl-w h/j/k/l` panels, `[t`/`]t` tabs, `Space n` new console, `Q` quit |
| Profiles | Explorer `n` create, `e` edit, `c` connect, `x` disconnect, `d` delete; `Tab`/`Shift-Tab` move form fields; `Esc` close |
| Explorer | `Space c` focus, `/` catalog search, `j/k` move, `h/l/Enter` collapse/expand, `r` refresh, `p` preview, `D` DDL |
| Editor Normal | `h/j/k/l`, `i/a/o`, `x`, `0/$`, `F5` scoped run, `Shift-F5` full run |
| Editor Insert | `Esc` or idle `Ctrl-c` Normal mode, Tab insert, `Ctrl-W/U/H`, arrows, Backspace/Delete |
| Results | `y` copies cell, `Y` copies row TSV, `Space Y` copies row with headers; Relation `D/p/o/r` switches/refreshes Data and DDL |

The footer and `?`/`F1` help show the active context. In Editor Normal mode `?`
is backward search; F1 and `Space ?` open help. Lowercase `q` is never a global
exit, so it remains available for future Vim macro semantics.

When application mouse capture is enabled, terminal-native text selection is
terminal-specific and commonly uses Shift-drag. Run with `--mouse off` when
terminal selection should take priority.

## Neovim Integration

The standalone [`yelog/lazydb.nvim`](https://github.com/yelog/lazydb.nvim)
plugin starts the `lazydb` executable in a floating terminal; it does not
contain the database engine itself. Install the CLI first using one of the
methods above, then add the plugin with one of the following methods. See the
plugin repository for its complete configuration and command reference.

### lazy.nvim

```lua
return {
  {
    "yelog/lazydb.nvim",
    cmd = {
      "LazyDB",
      "LazyDBToggle",
      "LazyDBHide",
      "LazyDBStop",
      "LazyDBRestart",
    },
    keys = {
      {
        "<leader>db",
        function()
          require("lazydb").toggle()
        end,
        desc = "Toggle LazyDB",
      },
    },
    opts = {
      executable = "lazydb",
      window = { width = 0.92, height = 0.90, border = "rounded" },
    },
  },
}
```

### Neovim Native Packages

For Neovim 0.10 and newer, install the plugin under the native package path.
Packages in `pack/*/start/*` are loaded automatically:

```bash
git clone https://github.com/yelog/lazydb.nvim.git \
  "${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/lazydb/start/lazydb.nvim"
```

Then configure it from `init.lua`:

```lua
require("lazydb").setup({ executable = "lazydb" })
```

Neovim 0.12 and newer also provides `vim.pack.add()`:

```lua
vim.pack.add({ "https://github.com/yelog/lazydb.nvim.git" })
require("lazydb").setup({ executable = "lazydb" })
```

For Neovim 0.10 and 0.11, use the `pack/*/start/*` method instead.

The plugin registers these commands:

```text
:LazyDB
:LazyDBToggle
:LazyDBHide
:LazyDBStop
:LazyDBRestart
```

Use `:checkhealth lazydb` to verify the executable and CLI API.

### Plugin Configuration

All supported plugin options are shown below. `executable` may be an absolute
path or a command available on `PATH`:

```lua
require("lazydb").setup({
  executable = "lazydb",
  window = { width = 0.92, height = 0.9, border = "rounded" },
})

vim.keymap.set("n", "<leader>db", function()
  require("lazydb").toggle()
end, { desc = "Toggle LazyDB" })
```

Use `:checkhealth lazydb`. The plugin does not map terminal-mode `q`, `Esc`,
`Ctrl-c`, `Ctrl-w`, or `Ctrl-\\`. Use `Ctrl-\\ Ctrl-n` to return to Neovim's
Terminal-Normal mode.

## Security Boundary

- Persisted profiles contain an explicit `none`, `prompt`, or `keyring`
  credential policy, never a password field or raw connection URL.
- Keyring references use `keyring:dev.lazydb.lazydb/<profile-uuid>`; the native
  service is `dev.lazydb.lazydb` and the account is the profile UUID.
- A `prompt` profile without a current session password opens the profile form
  instead of attempting an unauthenticated database connection.
- Delete removes the profile metadata and remembered keyring entry. Manual
  keyring cleanup is only needed for orphaned entries after external file edits.
- Imported passwords are wrapped in `secrecy` and remain process-local.
- Database error/value text is stripped of terminal control sequences.
- Dynamic SQL uses SQLx 0.9's explicit `AssertSqlSafe` marker and remains user
  SQL; internal catalog queries and generated identifiers are adapter-owned.
- SQLite read-only mode is enforced by the database open flags. PostgreSQL/MySQL
  read-only session settings are defense in depth, not a replacement for a
  least-privilege database account.
- Unknown database values degrade to an inert preview instead of panicking.

## Database Capabilities

See [database capabilities](docs/database-capabilities.md) for the driver
catalog matrix, lazy paging contract, relation snapshots, and version gates.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
