# Configuration

M0 loads connection profiles from the platform configuration directory:

- macOS: `~/Library/Application Support/dev.lazydb.lazydb/connections.toml`
- Linux: the `ProjectDirs` location derived from XDG configuration directories.

Use `lazydb --config /path/to/connections.toml` to override the profile file for
the current run. This option will point to a broader app configuration in a later
migration; M0 treats it as the connection-profile file.

## Startup Selection

If the profile file is empty, LazyDB opens a new Profile Manager form instead of
creating an implicit connection. `--profile NAME` selects a saved profile by
name. `--url URL` creates an ad-hoc profile for the current process and takes
precedence over `--profile`; it is never persisted.

`Space c` opens the manager while LazyDB is running. `Test Connection` does not
save metadata or change the active connection. `Save` persists the profile, and
`Save & Connect` persists it and activates the new pool.

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

## Persisted Profile Shape

Profiles are versioned TOML and contain no password field:

```toml
version = 1

[[profiles]]
id = "1c73c7c0-f944-4adc-a73a-1265fe1260a9"
name = "local-app"
kind = "postgres"
host = "127.0.0.1"
port = 5432
user = "app_user"
database = "app"
default_schema = "public"
ssl_mode = "require"
secret_ref = "keyring:dev.lazydb.lazydb/1c73c7c0-f944-4adc-a73a-1265fe1260a9"
read_only = false
environment = "development"
include_databases = []
include_schemas = []
```

`secret_ref` points to the native keyring service `dev.lazydb.lazydb` and account
equal to the profile UUID. The password is never stored in TOML. `Remember
Password` writes this entry to macOS Keychain or Linux Secret Service. If the
native store is unavailable, LazyDB falls back to a session-only password and
shows a warning. Delete removes the entry; manually edited or orphaned files
may require manual keyring cleanup.

An empty `include_databases` or `include_schemas` means all visible objects. TLS
mode maps to native driver modes. `verify-full`/`verify-identity` is recommended
for remote production connections.

## Read-only

`--read-only` overrides an ad-hoc profile for the current run.

- SQLite uses read-only database open flags.
- PostgreSQL configures `default_transaction_read_only=on`.
- MySQL configures each pooled session as transaction read-only.

For PostgreSQL and MySQL, use a database role with read-only grants as the actual
security boundary. Session settings can be changed by sufficiently privileged
SQL.
