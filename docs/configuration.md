# Configuration

This document is the reference for LazyDB's configuration surfaces. LazyDB has
four kinds of settings:

- **Application settings** are optional overrides in `settings.toml`. The
  complete built-in values live in [`config/default.toml`](../config/default.toml).
- **Command-line options** apply to the current process. Connection-selection
  options take precedence over saved-profile selection where stated below.
- **Connection profiles** are persisted in `connections.toml` and describe how
  LazyDB connects to a database.
- **Workspace state** is persisted separately in `workspace.toml` and the `sql/`
  directory. It stores open consoles and tabs, not credentials.

Unknown TOML fields are rejected. This helps catch spelling mistakes instead of
silently accepting a setting that LazyDB does not use.

Application settings use the following precedence, from highest to lowest:

1. Explicit command-line options
2. Values present in the user's `settings.toml`
3. The embedded [`config/default.toml`](../config/default.toml)

Tables are merged recursively, so a user file should contain only values that
depart from the defaults. Arrays, when introduced by a supported setting, are
replaced as a whole. LazyDB does not copy the complete defaults into the user
directory because doing so would prevent new defaults from being inherited on
upgrade.

## Configuration Files

`AppPaths` uses `~/lazydb/` for new users on macOS and Linux. Existing users
continue using the root selected by their installation during upgrades. Set
`LAZYDB_CONFIG_HOME` to override that directory, for example:

```bash
export LAZYDB_CONFIG_HOME=$HOME/lazydb
```

Windows continues to use the standard `%APPDATA%\\lazydb\\` directory.

`AppPaths` uses the following platform-specific user configuration directories:

| File | macOS | Linux | Windows | Purpose |
| --- | --- | --- | --- | --- |
| `connections.toml` | `~/lazydb/connections.toml` | `~/lazydb/connections.toml` | `%APPDATA%\\lazydb\\connections.toml` | Saved connection profiles |
| `credential.key` | Same directory as `connections.toml` | Same directory as `connections.toml` | Same directory as `connections.toml` | Device-local key for `local_encrypted` credentials |
| `workspace.toml` | `~/lazydb/workspace.toml` | `~/lazydb/workspace.toml` | `%APPDATA%\\lazydb\\workspace.toml` | Open profiles, consoles, and tabs |
| `sql/<UUID>.sql` | `~/lazydb/sql/<UUID>.sql` | `~/lazydb/sql/<UUID>.sql` | `%APPDATA%\\lazydb\\sql\\<UUID>.sql` | SQL text for persisted consoles |
| `settings.toml` | `~/lazydb/settings.toml` | `~/lazydb/settings.toml` | `%APPDATA%\\lazydb\\settings.toml` | Application settings |
| `install.json` | `~/lazydb/install.json` | `~/lazydb/install.json` | `%APPDATA%\\lazydb\\install.json` | Native installer ownership, channel, version, and target |

Native installation data also lives in this same application directory. The
`current` symlink selects the active version and `releases/<VERSION>/` contains
downloaded native releases. `LAZYDB_CONFIG_HOME` relocates all of these files
together on macOS and Linux.

`LAZYDB_CONFIG_HOME` applies to macOS and Linux only. Windows uses the standard
`%APPDATA%` roaming application-data directory. LazyDB creates private
configuration directories and writes profile files with owner-only permissions
on Unix. `--config PATH` overrides the complete
profile file for the current process; it does not relocate `credential.key` or
workspace state. On startup, LazyDB moves legacy files from
`~/Library/Application Support/dev.lazydb.lazydb/` and legacy native
installation data from `~/.local/share/lazydb/` into the selected config
directory when the destination entries do not already exist. This includes
`install.json`, `current`, and `releases/` for native installations.

The environment variable selects a different configuration root; it does not
copy profiles from the previous root. Existing roots are deliberately not
automatically merged. For example, after setting
`LAZYDB_CONFIG_HOME=$HOME/lazydb`, LazyDB reads connections from
`$HOME/lazydb/connections.toml`. Existing profiles under
`~/.config/lazydb/` remain there unless you explicitly migrate the complete
set of related files. When using `local_encrypted` credentials, migrate the
matching `credential.key` together with the profile file. Do not overwrite an
existing destination without first checking its contents.

To move a complete Unix application root, stop all LazyDB, MCP, and LSP
processes first and inspect the plan:

```bash
lazydb migrate-home --to "$HOME/lazydb" --dry-run
lazydb migrate-home --to "$HOME/lazydb" --yes
```

The migration only accepts a new destination on the same filesystem and does
not merge two existing roots. It keeps `~/.config/lazydb` as a compatibility
symlink after a successful move. If the source and destination are ambiguous,
select the source explicitly with `LAZYDB_CONFIG_HOME`.

The workspace stores open tabs in `workspace.toml` and SQL text in `sql/`.
LazyDB saves an empty welcome console as an empty workspace; a console with
user-entered SQL is treated as a draft and is not discarded merely because no
database profile is currently active.

If workspace saving fails while quitting, the UI offers retry, discard-and-quit,
or cancel-quit. Discarding applies to the current unsaved workspace snapshot;
it does not undo files that were successfully written by an earlier save.

## Command-Line Options

These options are global and can be placed before a subcommand. They are not
stored in `connections.toml`.

## Application Settings

Application settings are stored in `settings.toml` beside `connections.toml`.
The file may omit `version` and any table it does not override; omitted values
come from the embedded default configuration. The current schema version is
`1`. Dashboard monitoring uses a five-second interval by default. Change it
with:

```toml
[dashboard]
refresh_interval_seconds = 5
```

The value must be at least one second. The setting controls both metric and
process polling when the Dashboard is active.

New connections are global by default. To restrict newly saved connections to
the current project instead, set:

```toml
[connections]
default_access = "project"
```

This setting only affects newly created connections. Editing an existing
connection preserves its current access scope.

The complete schema and current values are best read directly in
[`config/default.toml`](../config/default.toml). Its sections are:

| Section | Settings |
| --- | --- |
| `terminal` | `mouse`, `color`, `clipboard.backend`, `clipboard.max_bytes` |
| `ui` | `icons`, `motion` |
| `execution` | `confirmation` |
| `connections` | `default_access` |
| `dashboard` | `refresh_interval_seconds` |
| `keybindings` | `preset`, `sequence_timeout_ms`, and grouped `global`, `leader`, `panes`, `explorer`, `results`, `editor`, `overlays` tables |

Only the `vim` keybinding preset is currently supported. It denotes the full
command and editor contract in [Keyboard Reference](keybindings.md); text-entry
keys and modalkit's core Vim editing operations are intentionally not separate
application settings. The grouped keybinding tables contain `help`, `omni`,
`quit`, `notification-history`,
`focus-next-pane`, `focus-previous-pane`, `run-statement`, `run-buffer`,
`next-tab`, `previous-tab`, and `close-tab`. Each value is a list of key
events or space-separated key sequences such as `F2`, `Ctrl-c`, `Shift-Tab`,
or `g t`; empty lists unbind a command. `omni` opens the global Omni Bar and
defaults to `F2`. The current Leader commands are
`open-dashboard`, `open-explorer`, `open-editors`, `run-leader-statement`,
`run-leader-buffer`, and `open-target-selector`.
The pane commands are `focus-pane-left`, `focus-pane-down`, `focus-pane-up`,
`focus-pane-right`, `toggle-pane-maximized`, and `reset-pane-sizes`.
The Explorer navigation commands are `explorer-move-down`, `explorer-move-up`,
`explorer-expand`, and `explorer-collapse`. The SQL Results grid commands are
`results-move-left`, `results-move-down`, `results-move-up`, and
`results-move-right`.
The remaining basic Explorer commands currently configurable are
`explorer-copy-selection`, `explorer-find`, `explorer-search`,
`explorer-new-profile`, `explorer-refresh`, and `explorer-toggle`.
Additional Results commands are `results-open-record`, `results-copy-cell`,
`results-copy-row`, `results-copy-row-headers`, `results-toggle-view`,
`results-first-column`, and `results-last-column`.
Results row alignment is configured by `results-align-middle`,
`results-align-top`, and `results-align-bottom`.
A key sequence remains active for 750 milliseconds by default, and
`sequence_timeout_ms` must be at least `1`.

| Option | Values / argument | Default | Description |
| --- | --- | --- | --- |
| `--config PATH` | File path | Platform profile path | Use another `connections.toml` for this run. Parent directories are created when saving. |
| `--profile NAME` | Profile name | None | Select a saved profile at startup. Ignored as the connection source when `--url` is also supplied, although the name is used for the ad-hoc profile when available. |
| `--url URL` | Connection URL | None | Open a session-only profile. It is never persisted and takes precedence over `--profile`. |
| `--read-only` | Flag | Off | Force an ad-hoc connection to read-only. For a saved profile, the stored profile setting remains unchanged; adapter behavior is described below. |
| `--mouse MODE` | `auto`, `on`, `off` | `auto` | Enable mouse input automatically, force it on, or disable it. |
| `--color MODE` | `auto`, `always`, `never` | `auto` | Select automatic, forced, or disabled terminal color output. |
| `--icons MODE` | `nerd-font`, `unicode`, `ascii` | `nerd-font` | Select branded Nerd Font glyphs, standard Unicode fallbacks, or ASCII-only output. |
| `--theme-file PATH` | Theme-file-v1 JSON path | None | Load a complete external theme and watch it for changes. Ignored when color is `never`. |
| `--motion MODE` | `full`, `reduced`, `off` | `full` | Select full loading animation, reduced animation, or no animation. |
| `--confirm-execution POLICY` | `risky`, `always` | `risky` | Confirm only risky SQL statements, or confirm every execution. |

## External Theme File

`--theme-file PATH` loads a `theme-file-v1` JSON document at startup and polls
for replacements while LazyDB runs. It must contain `"version": 1` and a
`"colors"` object with all semantic UI and SQL syntax color keys. Values are
`"default"` or six-digit `#rrggbb` colors. Unknown or missing fields,
unsupported versions, invalid colors, unreadable or non-regular paths, and
input larger than 16 KiB are rejected.

Invalid or unavailable files preserve the last valid theme, or the built-in
theme if none has loaded. `--color never` takes precedence and uses the plain
built-in theme without reading or watching the file. A valid replacement is
applied without restarting the TUI; polling means it is not necessarily
visible immediately.

The standalone `lazydb.nvim` plugin follows this protocol by default. With
`theme.follow = true`, it probes `lazydb capabilities --json`, publishes a
private owner-only snapshot atomically, and passes `--theme-file` to LazyDB.
`theme.follow = false` skips the probe. `:LazyDBRefreshTheme` and
`require("lazydb").refresh_theme()` publish current Neovim highlights
immediately; ColorScheme and background changes are coalesced automatically.
Probe, permission, publication, and unsupported-CLI failures fall back to the
built-in theme and leave the session running. The result remains limited by
Neovim highlight groups and terminal color support.

Clipboard output uses the system clipboard by default. To explicitly send copy
operations as OSC 52 to the terminal, or disable clipboard writes, set:

```toml
[terminal.clipboard]
backend = "osc52" # "system", "osc52", or "off"
max_bytes = 1000000
```

`max_bytes` limits the base64-encoded OSC 52 payload. Oversized payloads are
rejected without truncation. OSC 52 is write-only and has no reliable delivery
confirmation, so successful sends are reported as sent to the terminal
clipboard. The system backend does not fall back to OSC 52, and no tmux
passthrough or automatic backend selection is performed. The terminal must
support OSC 52 and may need its own permission setting; SSH and tmux forwarding
are environment-dependent and are not established by LazyDB.

Clipboard copy actions are explicit: keyboard commands and view buttons issue a
copy request, while mouse drag previews and automatically copies text in
supported text views when the button is released. The preview then disappears.
Dragging does not copy grid cells, rows, form fields, or selector text.
The complete-value paths are the selected grid cell/row commands, Record View's
cell/row buttons, and the text-detail view's `Copy all` button. Visual mode
selection remains a separate persistent editor state; mouse drag is a one-shot
copy gesture.

The default `Ctrl-c` quit binding is unchanged. `Ctrl-Shift-s` is the separate
global command that releases mouse capture for terminal-native selection; it is
not a clipboard write and `Esc` restores capture.

`--color`, `--mouse`, `--icons`, `--motion`, and `--confirm-execution` override
`settings.toml` for the current process. The `--config` option is also accepted by agent and
MCP commands so they read the same profile set.

Subcommand-specific options are not connection-file settings. They include
`update --channel stable|beta`, agent `--project`, `--connection`, `--limit`,
`--sql`, `--file`, `--write-policy`, and MCP `serve --project`, `--connection`,
and `--write-policy`. The MCP and agent execution write policy defaults to
`deny`; see [Coding-Agent Database Access](coding-agent-access.md).

## Connection Profile Fields

The current profile file format is version `5`:

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `id` | UUID string | Generated on creation | Stable identity used by workspace state and system credential references. Must be unique within the file. |
| `name` | String | Derived from the database, host, or SQLite filename | Name shown in the Explorer and accepted by `--profile` and agent `--connection`. |
| `access` | Table | `{ scope = "global" }` | Controls which projects can see a saved profile. |
| `kind` | `postgres`, `mysql`, `mariadb`, `oracle`, `sqlserver`, `sqlite` | Required | Database adapter to use. Oracle uses the native driver included in the default build and requires Oracle Instant Client at runtime. |
| `url_format` | Kebab-case enum | Driver-specific | URL spelling used when LazyDB displays or regenerates the connection URL. |
| `host` | String or `null` | `null` | Server hostname or address. PostgreSQL, MySQL/MariaDB, and SQL Server use this field. |
| `port` | Integer or `null` | Driver default when imported | Server port. PostgreSQL defaults to `5432`, MySQL/MariaDB to `3306`, and SQL Server to `1433`. SQL Server profiles require an explicit TCP port when connecting. |
| `user` | String or `null` | `null` | Server login name. SQL Server uses SQL username/password authentication; SQLite does not use it. |
| `database` | String or `null` | `null` | Database name for PostgreSQL, MySQL/MariaDB, or SQL Server, or the logical SQLite path value. |
| `default_schema` | String or `null` | `null` | PostgreSQL `currentSchema`, SQL Server schema, or SQLite `main`. MySQL has no separate default-schema field. |
| `sqlite_path` | Path or `null` | `null` | SQLite file path. It is `null` for an in-memory database. |
| `ssl_mode` | `disable`, `prefer`, `require`, `verify-ca`, `verify-full` | `prefer` | TLS policy for PostgreSQL, MySQL, and SQL Server. SQLite always uses `disable`. |
| `credential_policy` | Tagged table | `{ policy = "none" }` | Where the password comes from. |
| `read_only` | Boolean | `false` | Requests adapter-level read-only behavior. Use database grants for authorization. |
| `environment` | `development`, `staging`, `production` | `development` | Environment label used by the UI and agent write-policy checks. |
| `catalog_scope` | Table | Derived from database and schema | Databases and schemas visible to Explorer and completion. |

Oracle profiles use the native Oracle client. LazyDB first checks the
`LAZYDB_ORACLE_CLIENT_LIB_DIR` environment variable, then
`~/.local/share/lazydb/oracle/current`, and finally the Oracle client's normal
library search paths. The selected directory must contain the ARM64
`libclntsh.dylib` on Apple Silicon. Client discovery is process-wide; changing
the client directory requires restarting LazyDB.

`url_format` accepts `postgres`, `postgresql`, `jdbc-postgresql` for PostgreSQL;
`mysql`, `jdbc-mysql` for MySQL; `mariadb` for MariaDB; `jdbc-oracle` for Oracle; `sqlserver`, `mssql`, `jdbc-sqlserver` for SQL
Server; and `sqlite`, `file-uri`, `jdbc-sqlite` for SQLite. Defaults are
`postgresql`, `mysql`, `sqlserver`, and `sqlite` respectively.

### Profile Access

Global profiles use `access = { scope = "global" }`. Project-scoped profiles use
absolute, canonical project roots:

```toml
access = { scope = "projects", roots = ["/Users/alice/src/orders"] }
```

The current project is the nearest Git root above the startup directory. Project
scope controls organization and discovery, not database authorization.

### Credential Policy

The `credential_policy` table is tagged by `policy`:

| Value | Example | Behavior |
| --- | --- | --- |
| `none` | `{ policy = "none" }` | Connect without a stored password. |
| `prompt` | `{ policy = "prompt" }` | Ask for a password for the current process. |
| `local_encrypted` | `{ policy = "local_encrypted", reference = { version = 1, nonce = "...", ciphertext = "..." } }` | Store authenticated ciphertext in `connections.toml`; the key is in `credential.key`. |
| `system` | `{ policy = "system", reference = "<profile UUID>" }` | Use macOS Login Keychain or Linux Secret Service. |

`keyring` may appear in legacy version 3 files and is normalized to `system` on
load. Do not hand-edit encrypted values or copy only `connections.toml` and
expect local-encrypted passwords to work; the key file is required too.

### Catalog Scope

`catalog_scope` contains a `databases` selection. Each selection is either `all`
or `selected`:

```toml
catalog_scope = { databases = { mode = "selected", items = [
  { name = "orders", schemas = { mode = "selected", items = ["public", "audit"] } },
  { name = "reporting", schemas = { mode = "all" } },
] } }
```

The unrestricted form is `catalog_scope = { databases = { mode = "all" } }`.
Selected names must be non-empty and unique. A new PostgreSQL or SQLite profile
selects its database; a PostgreSQL `default_schema` narrows that selection.
MySQL treats each selected database as its own schema and has no separate schema
toggle.

## Workspace File

Workspace state is not an application-settings file. Its current format is
version `3` and contains `version`, optional `active_profile`, and `profiles`.
Each profile workspace contains `profile_id`, optional `active_tab`, `consoles`,
and `tabs`. A console contains `id`, `name`, `sql_file`, optional `target`,
`transaction_mode`, and `open` (default `true`). A tab is either a console tab
or a relation tab with its relation identity, qualified name, catalog kind, title,
and view. SQL contents are stored in the referenced `sql/UUID.sql` file.
Workspace files do not contain passwords.

## Installation Updates

`lazydb update --check` checks the configured channel without changing the
installation. `lazydb update` applies a newer release only when the executable
is a native Pages installation; it verifies the channel manifest and archive
checksum before switching the active version. `--channel stable` and
`--channel beta` select the channel for that operation. Without the flag,
LazyDB uses the channel recorded by the native installer, falling back to
stable.

Installations owned by Homebrew, Debian, RPM, Arch, or Cargo are not
replaced by LazyDB. The command reports the corresponding manager action:
`brew upgrade yelog/tap/lazydb`, `sudo apt install --only-upgrade ./lazydb_VERSION_ARCH.deb`,
`sudo dnf upgrade ./lazydb_VERSION_ARCH.rpm`,
`sudo pacman -U ./lazydb_VERSION_ARCH.pkg.tar.zst`, or `cargo install lazydb`.
Use the owning manager for the actual update. These Linux packages are direct
Release assets, not configured distribution repositories. npm-managed files are
detected and protected from overwrite, but official npm distribution is
currently unavailable; use the Pages installer or Homebrew instead.

M0 loads connection profiles from the platform configuration directory:

- macOS/Linux: `$HOME/lazydb/connections.toml` by default, or
  `$LAZYDB_CONFIG_HOME/connections.toml` when the environment variable is set
- Windows: `%APPDATA%\\lazydb\\connections.toml`

Use `lazydb --config /path/to/connections.toml` to override the profile file for
the current run. This option will point to a broader app configuration in a later
migration; M0 treats it as the connection-profile file.

## Icon Mode

`--icons` selects the icon set for the current process:

```text
lazydb --icons nerd-font   # default; branded database and object glyphs
lazydb --icons unicode     # standard Unicode fallback
lazydb --icons ascii       # maximum compatibility
```

Nerd Fonts 3.x or a compatible Symbols Nerd Font fallback is recommended for
the branded PostgreSQL, MySQL, and SQLite glyphs. LazyDB does not detect or
install fonts, and the option is not stored in `connections.toml`. When using
SSH, glyph rendering depends on the font configured by the local terminal.

## Project-Scoped Connections

Saved profiles are stored in the user-level `connections.toml`. Profile file
version 5 adds connection access metadata. A profile is either global or
project-scoped with a list of canonical project roots; old profile versions
migrate to global. New saved profiles created from a project default to that
project. Project association controls Explorer organization, not database
authorization.

LazyDB identifies the current project from the nearest Git root above the
startup directory. A `.git` directory or file is accepted. Outside Git, the
canonical startup directory is used. `--config` still overrides the complete
profile file, including access metadata.

The Explorer shows current-project, global, and session connections directly.
Other project-scoped connections are under a collapsed `OTHERS` group and keep
all normal connection and catalog actions. Select a saved connection and press
`s` to open Connection access. Removing the last association leaves the
connection project-scoped and unassigned under `OTHERS`.

## Startup Selection

If the profile file is empty, LazyDB opens a new Profile Manager form instead of
creating an implicit connection. The Explorer contains a `No profiles` row
(`EmptyProfiles`) whose primary action starts that form. There is no profile-list
popup. `--profile NAME` selects a saved profile by name at startup. `--url URL`
creates an ad-hoc session profile for the current process and takes precedence
over `--profile`; it is never persisted.

`Space c` focuses the connection Explorer while LazyDB is running. The Explorer keeps roots by
profile UUID. Temporary roots are marked `SESSION`; saved roots have no
provenance label. Connection status is shown with a compact marker, with text
retained for connecting, syncing, and failed states. Refresh, expand, retry, and relation
actions target the selected UUID. `Test Connection` does not save metadata or
change the active connection; it probes and discovers databases/schemas for the
draft's scope picker. `Save` persists the profile, and `Save & Connect` persists
it and activates the new pool.

## Ad-hoc Connections

`--url` accepts:

```text
sqlite::memory:
sqlite:///absolute/path/database.db
postgresql://user@host:5432/database?sslmode=require
jdbc:postgresql://host:5432/database?currentSchema=tools
mysql://user@host:3306/database?sslMode=REQUIRED
jdbc:mysql://host:3306/database?useSSL=true
sqlserver://<username>:<password>@<host>:1433/<database>?encrypt=true&trustServerCertificate=false
mssql://<username>:<password>@<host>:1433/<database>?encrypt=true&trustServerCertificate=true
jdbc:sqlserver://<host>:1433;databaseName=<database>;user=<username>;password=<password>;encrypt=true;trustServerCertificate=false
```

SQL Server URLs use SQL username/password authentication and an explicit TCP
port. The `sqlserver://` and `mssql://` forms accept `schema` or
`currentSchema`, `encrypt`, `trustServerCertificate`, and `readOnly` query
parameters. JDBC uses the corresponding semicolon properties. Windows
Integrated Authentication, Kerberos, Entra authentication, Named Instance or
SQL Browser properties are not supported yet. The examples use placeholders;
do not put real passwords in command-line arguments.

Supply a session password through `LAZYDB_PASSWORD`. It is read at startup only,
bound to the selected profile, and is not reused for a different profile. Avoid
URL passwords because process arguments can be inspected by other local users.

The Profile Manager also accepts these forms in its URL field. Changing the URL
atomically fills the driver, host, port, user, database, default schema, SSL, and
read-only settings. Changing those settings regenerates a password-free URL.
Passwords parsed from URLs are moved into the secret-backed Password field and
are never retained in the displayed URL.

## Password Storage

New PostgreSQL and MySQL connections save entered passwords by default using
`LOCAL ENCRYPTED`. The password is authenticated-encrypted in
`connections.toml`; the separate device-local `credential.key` file contains
the encryption key. This default works in desktop and headless Linux sessions,
but copying only `connections.toml` does not copy the password. The local mode
protects against accidental disclosure, not a process running as the same OS
user with access to the complete LazyDB configuration directory.

When supported by the current session, the Profile Manager also offers
`MACOS LOGIN KEYCHAIN` or `SECRET SERVICE`. A locked provider remains visible as
`LOCKED`; a missing provider or session bus is not shown for new connections. If
writing a selected System provider fails, LazyDB falls back to Local Encrypted
storage and reports the actual storage mode. It does not keep a hidden local copy
for a successfully stored System credential.

## Persisted Profile Shape

Profiles are versioned TOML. Local encrypted profiles contain authenticated
ciphertext rather than a plaintext password:

```toml
version = 5

[[profiles]]
id = "1c73c7c0-f944-4adc-a73a-1265fe1260a9"
name = "local-app"
kind = "postgres"
url_format = "postgre-sql"
host = "127.0.0.1"
port = 5432
user = "app_user"
database = "app"
default_schema = "public"
ssl_mode = "require"
credential_policy = { policy = "local_encrypted", reference = { version = 1, nonce = "...", ciphertext = "..." } }
read_only = false
environment = "development"
catalog_scope = { databases = { mode = "selected", items = [{ name = "app", schemas = { mode = "selected", items = ["public"] } }] } }
```

Credential storage is explicit: `none` permits a passwordless connection,
`prompt` requires a current-process password, `local_encrypted` stores an
authenticated ciphertext in this file, and `system` references the native
credential service using the profile UUID as account. Local encryption uses a
random device-local key stored separately as `credential.key`. New PostgreSQL
and MySQL connections default to `LOCAL ENCRYPTED`, so saved passwords survive
restart on desktop and headless systems. `SYSTEM` is offered only when macOS
Login Keychain or Linux Secret Service is detected; a locked provider is shown
as `LOCKED`. If a System write fails, LazyDB saves using Local Encrypted storage
and reports the actual fallback. Copying only this file does not copy the local
encryption key.

The current profile serializer stores scope under `catalog_scope`, with
`databases` and `schemas` represented as `All` or `Selected` lists. PostgreSQL
and SQLite expose database then schema rows. MySQL treats each selected database
as its own schema and mirrors that name in the picker; the mirrored schema
cannot be toggled separately. If discovery is stale or unavailable, saved
selections remain visible with a warning.

PostgreSQL's optional `default_schema` is also the default derived visibility
scope. If it is non-empty and Visible Objects has not been customized, Explorer
loads only that database/schema. Clearing it allows all schemas in the selected
database. Explicit Visible Objects selections are preserved and must include the
configured default schema. MySQL has no separate default-schema field; SQLite
uses `main` by default.

TLS mode maps to native driver modes. For SQL Server, `disable` requests
plaintext, `require` enables encryption while trusting the server certificate,
and `verify-ca`/`verify-full` enable encryption with certificate verification.
`prefer` is the default and fails closed as verified TLS in the native TDS
adapter. `verify-full`/`verify-identity` is recommended for remote production
connections. SQL Server cancellation closes the active session; SQL Server rolls
back any open transaction on that session.

## Read-only

`--read-only` overrides an ad-hoc profile for the current run.

- SQLite uses read-only database open flags.
- PostgreSQL configures `default_transaction_read_only=on`.
- MySQL configures each pooled session as transaction read-only.
- SQL Server configures the session as read-only where supported by the driver.

For PostgreSQL and MySQL, use a database role with read-only grants as the actual
security boundary. Session settings can be changed by sufficiently privileged
SQL.

## Coding Agents

LazyDB's coding-agent interfaces expose current-project and global profiles, but
hide profiles assigned only to other projects. See
[`coding-agent-access.md`](coding-agent-access.md) for JSON CLI, stdio MCP,
Codex, OpenCode, and Claude Code configuration. The MCP server defaults to
`--write-policy deny`; client-side MCP approval settings are an additional layer
and cannot relax LazyDB profile or database permissions. For writable
development or staging sessions, use `--write-policy non-production` and retain
per-call client approval. Restart the MCP client after changing the command.

## Update Checks

LazyDB checks for releases asynchronously after the first frame when startup
checking is enabled. Successful checks are cached for 24 hours by default; failed
checks are not cached.

```toml
[updates]
check_on_startup = true
check_interval_hours = 24
```

Press `F9` or click the version/update badge in the header to open the Update Center.
Only confirmed native installations can be updated directly. Package-manager and
source installations are never overwritten and instead show their upgrade guidance.
After a native update is installed, the current process continues running normally;
the UI distinguishes `Running` from `Installed` until the user chooses `Restart now`.
