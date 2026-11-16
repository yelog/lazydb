# Isolated native data operation acceptance

Run real PostgreSQL, MySQL, and MariaDB backup, restore, and copy operations:

```sh
bash tests/fixtures/data_ops/run.sh
```

Select engines when needed:

```sh
bash tests/fixtures/data_ops/run.sh postgres
bash tests/fixtures/data_ops/run.sh mysql mariadb
```

Prerequisites are Docker through a local Unix endpoint, Bash, Python 3, Cargo,
and cached `postgres:16`, `mysql:8.4`, and/or `mariadb:11.4` images for the selected
engines. The runner uses `--pull=never`. No native database clients are required
on the host.

Each run allocates a private temporary root, a random fixture ID, three
available random ports, and uniquely named containers. Selected servers listen
on the same internal and host port, published only on `127.0.0.1`. The temporary
root is mounted at the identical absolute path inside every test container.
`TMPDIR` points there while Cargo runs, so native providers' private credential
files and archives are visible to container clients.

The explicit `NativeToolPaths` point to wrappers in the fixture root. Wrappers
execute the image's actual clients using `docker exec -i`, preserving arguments
and stdin/stdout. They forward only the listed libpq connection variables,
including the `PGPASSFILE` path, and use the provider's private home. They never
forward `PGPASSWORD`. The wrapper configuration records the Docker binary,
local endpoint, and fixture container names because native client execution
clears the inherited environment. MySQL/MariaDB option-file arguments remain
unchanged and resolve through the shared mount.

Passwords are generated for the disposable servers, stored in private fixture
environment files, and supplied through `.with_passwords`. Test profiles are
imported from password-free fixture URLs, use disabled TLS and all catalogs,
and allow writes. Each engine gets a fresh job store. The test never reads
saved profiles or requests credentials from the native secret store.

The exit/signal trap removes only container IDs created by this run whose
ownership label matches its fixture ID, their anonymous volumes, and the
matching private root. It never prunes Docker resources or interacts with
existing containers or databases.

Each engine's test verifies:

- A real full native backup of 12,003 rows, with a persisted manifest and
  matching archive byte count and SHA-256.
- Restore into a new database and copy into another new database, retaining
  the completed copy archive and metadata.
- Row counts, Unicode and quoted text, NULL versus empty values, binary data,
  exact large decimal and signed integer values, floating point and boolean
  values, column types, and an additional table. PostgreSQL also covers a view
  and another schema. MySQL/MariaDB fixtures use tables: ordinary view dumps can
  qualify references with the source database, which the provider intentionally
  rejects for a new destination and requires manual migration.
- The next inserted ID is 15,000 in each new database, preserving sequence or
  auto-increment state beyond the largest existing row ID.
- Refusal to replace an old backup archive, restore into an existing database,
  or copy into an existing database with a fresh archive path.
- A copy whose real dump succeeds and whose restore client is deliberately
  failed. The wrapper requires the completed backup manifest to exist before
  the restore attempt fails. The failed job must retain its completed backup
  outcome with matching size and SHA-256; the retained archive must restore
  successfully into another new database with the same row/type/next-ID checks.
- Four successful persisted job records and one expected failed copy record
  without fixture passwords, and an unchanged source row count.

The test environment contract is private to this fixture:

| Variable | Meaning |
| --- | --- |
| `LAZYDB_DATA_OPS_FIXTURE_ROOT` | Root with `fixture-id`, engine port files, wrappers, and private runtime files |
| `TMPDIR` | The same canonical absolute fixture root |
| `LAZYDB_DATA_OPS_POSTGRES_URL` | Password-free fixture PostgreSQL URL |
| `LAZYDB_DATA_OPS_POSTGRES_PASSWORD` | Throwaway PostgreSQL password |
| `LAZYDB_DATA_OPS_POSTGRES_DUMP` / `_RESTORE` | Explicit `pg_dump` / `pg_restore` wrapper paths |
| `LAZYDB_DATA_OPS_MYSQL_URL` | Password-free fixture MySQL URL |
| `LAZYDB_DATA_OPS_MYSQL_PASSWORD` | Throwaway MySQL password |
| `LAZYDB_DATA_OPS_MYSQL_DUMP` / `_RESTORE` | Explicit `mysqldump` / `mysql` wrapper paths |
| `LAZYDB_DATA_OPS_MARIADB_URL` | Password-free fixture MariaDB URL |
| `LAZYDB_DATA_OPS_MARIADB_PASSWORD` | Throwaway MariaDB password |
| `LAZYDB_DATA_OPS_MARIADB_DUMP` / `_RESTORE` | Explicit `mariadb-dump` / `mariadb` wrapper paths |

The runner exports these values only to its child test process. Do not print
the password variables or generated `.env` files. Running
`cargo test --locked --test data_ops_servers -- --nocapture` without fixture
URLs prints an explicit `SKIP` message for every absent engine. Such an `ok`
result is a skipped optional test and is not evidence of native acceptance.
Native provider success warnings about archive verification limits remain
valid; these tests establish parity only for the seeded fixture and topology.

## SQLite real PTY smoke

With a freshly built binary:

```sh
python3 tests/fixtures/data_ops/sqlite-tui.py --binary /absolute/path/to/debug/lazydb
```

This separate smoke uses Python 3's standard library and the repository's
existing PTY screen reader. It creates a private configuration, project, saved
SQLite profile, and three-row database. Real keyboard input opens F2 → Backup
Database, fills the artifact field, reviews the immutable plan, executes it,
and reopens Data Tasks through F2. It checks the backup's integrity, rows,
NULL/empty/binary/numeric values, sequence, manifest checksum, and persisted
task, then verifies clean process and terminal exit. The private fixture is
removed even on failure; no user configuration or existing database is used.

## Recorded acceptance

On November 16, 2026, the final complete Docker runner passed all three real
native tests after the completed-backup checkpoint and bounded metadata reader
changes: PostgreSQL 16, MySQL 8.4, and MariaDB 11.4. Cargo reported `3 passed; 0
failed`, with 14.34 seconds in the test executable. Across the three engines this
covered 15 persisted jobs: 12 successful jobs and three deliberately failed copy
jobs. The restore wrappers required each backup manifest to exist before the
injected failure; all three failed jobs retained their backup outcome, archive,
and manifest, and the retained archives restored successfully. Nine restored,
copied, or recovered databases passed the row/type/next-ID checks, and all nine
overwrite refusals passed. The owned containers and fixture root were removed.

An earlier baseline run also passed `3 passed; 0 failed` in 9.38 seconds, covering
nine successful backup/restore/copy jobs, six verified destination databases,
and nine overwrite refusals before the failed-copy checkpoint coverage was added.

The separate SQLite real PTY smoke was rerun against the freshly built binary
after the checkpoint and metadata reader changes. All ten checks passed,
including F2 command selection, draft/review/execute transitions, task history,
backup file and manifest verification, source preservation, and clean terminal
exit. Its private configuration and SQLite files were removed.
