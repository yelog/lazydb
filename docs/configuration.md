# Configuration

M0 loads connection profiles from the platform configuration directory:

- macOS: `~/Library/Application Support/dev.lazydb.lazydb/connections.toml`
- Linux: the `ProjectDirs` location derived from XDG configuration directories.

Use `lazydb --config /path/to/connections.toml` to override the profile file for
the current run. This option will point to a broader app configuration in a later
migration; M0 treats it as the connection-profile file.

## Startup Selection

If the profile file is empty, LazyDB opens a new Profile Manager form instead of
creating an implicit connection. The Explorer contains a `No profiles` row
(`EmptyProfiles`) whose primary action starts that form. There is no profile-list
popup. `--profile NAME` selects a saved profile by name at startup. `--url URL`
creates an ad-hoc session profile for the current process and takes precedence
over `--profile`; it is never persisted.

`Space c` opens the manager while LazyDB is running. The Explorer keeps roots by
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

## Persisted Profile Shape

Profiles are versioned TOML and contain no password field:

```toml
version = 3

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
credential_policy = { policy = "keyring", reference = "keyring:dev.lazydb.lazydb/1c73c7c0-f944-4adc-a73a-1265fe1260a9" }
read_only = false
environment = "development"
catalog_scope = { databases = { mode = "selected", items = [{ name = "app", schemas = { mode = "selected", items = ["public"] } }] } }
```

Credential policy is explicit: `none` permits a passwordless connection,
`prompt` requires a current-process password, and `keyring` references the
native service `dev.lazydb.lazydb` with the profile UUID as account. The password
is never stored in TOML. New PostgreSQL and MySQL forms enable `Remember
Password` by default and write the keyring entry; disable it to keep a password
for the current session only. If the native store is unavailable, LazyDB
persists `prompt`, keeps the password only for the current session, and shows a
warning. A later restart opens the profile form for a password instead of trying
an empty password. Delete removes a keyring entry; externally edited or orphaned
files may require manual cleanup.

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
