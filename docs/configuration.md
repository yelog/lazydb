# Configuration

M0 loads connection profiles from the platform configuration directory:

- macOS: `~/Library/Application Support/dev.lazydb.lazydb/connections.toml`
- Linux: the `ProjectDirs` location derived from XDG configuration directories.

Use `lazydb --config /path/to/connections.toml` to override the profile file for
the current run. This option will point to a broader app configuration in a later
migration; M0 treats it as the connection-profile file.

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

Supply a session password through `LAZYDB_PASSWORD`. Avoid URL passwords because
process arguments can be inspected by other local users.

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
secret_ref = "keyring:lazydb/local-app"
read_only = false
environment = "development"
include_databases = []
include_schemas = []
```

`secret_ref` is persisted for the upcoming keyring integration. M0 does not yet
resolve it, so authenticated persisted profiles also require the session-only
`LAZYDB_PASSWORD` environment variable.

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
