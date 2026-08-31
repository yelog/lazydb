# Configuration

M0 loads connection profiles from the platform configuration directory:

- macOS: `~/Library/Application Support/dev.lazydb.lazydb/connections.toml`
- Linux: the `ProjectDirs` location derived from XDG configuration directories.

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
```

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
credential_policy = { policy = "local_encrypted", value = { version = 1, nonce = "...", ciphertext = "..." } }
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

TLS mode maps to native driver modes. `verify-full`/`verify-identity` is
recommended for remote production connections.

## Read-only

`--read-only` overrides an ad-hoc profile for the current run.

- SQLite uses read-only database open flags.
- PostgreSQL configures `default_transaction_read_only=on`.
- MySQL configures each pooled session as transaction read-only.

For PostgreSQL and MySQL, use a database role with read-only grants as the actual
security boundary. Session settings can be changed by sufficiently privileged
SQL.

## Coding Agents

LazyDB's coding-agent interfaces expose current-project and global profiles, but
hide profiles assigned only to other projects. See
[`coding-agent-access.md`](coding-agent-access.md) for JSON CLI, stdio MCP,
Codex, OpenCode, and Claude Code configuration. The MCP server defaults to
`--write-policy deny`; client-side MCP approval settings are an additional layer
and cannot relax LazyDB profile or database permissions.
