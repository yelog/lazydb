# LazyDB

LazyDB is a keyboard-first, mouse-capable terminal database IDE written in Rust.
It aims to bring the core JetBrains Database Tools workflow to a fast,
standalone TUI with a distinctive deep-space visual system and a thin Neovim
integration.

> Project status: M0 runnable foundation. The current build is useful for
> connection, catalog, query, preview, and DDL exploration, but it is not yet a
> production-ready replacement for DataGrip.

## What Works

- PostgreSQL, MySQL, and SQLite connection and server probing.
- PostgreSQL/MySQL/SQLite URL import plus JDBC PostgreSQL/MySQL import.
- SQLite tables, views, columns, indexes, foreign keys, triggers, and DDL.
- PostgreSQL/MySQL databases, schemas, tables, views, columns, indexes, and
  foreign keys.
- Multiple SQL console tabs with a compact Vim-style Normal/Insert editor.
- Multi-statement execution, typed result decoding, Output history, and
  generation-safe asynchronous updates.
- Runtime profile manager for creating, testing, saving, editing, deleting, and
  switching PostgreSQL, MySQL, and SQLite connections.
- Optional remembered passwords in the native system keyring, with explicit
  session-only fallback when the keyring is unavailable.
- Object tree refresh, table preview with a 500-row limit, and DDL in a new tab.
- Responsive 80-column focus mode, standard split layout, truecolor theme,
  contextual help, bounded TachyonFX transitions, and mouse hit regions.
- A thin `lazydb.nvim` floating-terminal plugin with one process per Neovim tab.
- Stable machine-readable `version`, `capabilities`, and `doctor` commands.

## Deliberately Deferred

The following items are in M1/M2 and have no fake controls in the current UI:

- Persistent console recovery and renaming.
- Current-statement/visual-selection execution.
- Where/Order By controls and selectable paging sizes.
- Manual transaction sessions and commit/rollback controls.
- Staged grid editing, optimistic conflict detection, insert, and delete.
- Semantic completion, SQL formatting, plans, native cancellation, SSH, import,
  and export.

See [the product design](docs/plans/2026-08-24-lazydb-design.md) and
[M0 implementation plan](docs/plans/2026-08-24-lazydb-m0-implementation.md).

## Build

Requirements:

- Rust 1.94 or newer.
- macOS or Linux.
- A UTF-8 terminal. Truecolor is recommended.

```bash
cargo build --release
```

Run an in-memory SQLite workspace:

```bash
cargo run -- --url sqlite::memory:
```

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

On first launch with no saved profiles, LazyDB opens a new Profile Manager form.
Press `Space c` from any normal-mode workspace to open the manager later. Use
`Test Connection` to validate a draft without persistence, `Save` to persist
metadata, and `Save & Connect` to persist and activate it. `Remember Password`
stores only in the native macOS Keychain or Linux Secret Service; if that store
is unavailable, LazyDB reports a session-only downgrade. Passwords in URLs are
not supported as a safe credential mechanism.

## CLI Contract

```text
lazydb [--config PATH] [--profile NAME] [--read-only]
       [--mouse auto|on|off] [--color auto|always|never]

lazydb version --json
lazydb capabilities --json
lazydb doctor --json [--profile NAME]
```

`--url` creates an ad-hoc connection and never writes a profile. `--profile`
selects a saved profile by name; if both are supplied, `--url` wins.

## Essential Keys

| Context | Keys |
| --- | --- |
| Global | `F1` help, `Ctrl-w h/j/k/l` panels, `[t`/`]t` tabs, `Space n` new console, `Q` quit |
| Profiles | `Space c` open manager; `j/k` select; `Enter` connect/edit; `n` new; `t` test; `s` save; `d` delete; `Esc` close |
| Explorer | `j/k` move, `h/l/Enter` collapse/expand, `r` refresh, `p` preview, `D` DDL |
| Editor Normal | `h/j/k/l`, `i/a/o`, `x`, `0/$`, `F5` or `Space r` run |
| Editor Insert | `Esc` or idle `Ctrl-c` Normal mode, Tab insert, arrows, Backspace/Delete |
| Results | `h/j/k/l` cell movement, `o` switch Data/Output |

The footer and `?`/`F1` help show the active context. Lowercase `q` is never a
global exit, so it remains available for future Vim macro semantics.

## Neovim

Add `lazydb.nvim` as a local plugin and point it at the built executable:

```lua
require("lazydb").setup({
  executable = vim.fn.expand("~/path/to/lazydb/target/release/lazydb"),
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

- Persisted profile models contain `secret_ref`, never a password field.
- `secret_ref` uses `keyring:dev.lazydb.lazydb/<profile-uuid>`; the native
  keyring service is `dev.lazydb.lazydb` and the account is the profile UUID.
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

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
