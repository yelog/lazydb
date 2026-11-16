# Backups, Restores, and Database Copies

LazyDB provides native full-database backup, restore into a **new** target,
and same-engine database copy. The TUI and `lazydb data` commands share
preflight, execution, cancellation, verification, and task history.

These operations read the database independently of the data grid and its
preview limits. Table-level file import/export, cross-engine conversion,
incremental backups, and continuous synchronization are separate, future
workflows.

## Supported Engines

| Engine | Backup | Restore | Requirements |
| --- | --- | --- | --- |
| PostgreSQL | Custom-format `pg_dump` archive | `pg_restore` into a new database | Compatible `pg_dump` and `pg_restore` on PATH or explicit tool paths |
| MySQL | Native SQL dump | Native `mysql` client into a new database | Compatible `mysqldump` and `mysql` |
| MariaDB | Native SQL dump | Native `mariadb` client into a new database | Compatible `mariadb-dump` and `mariadb` |
| SQLite | Online `VACUUM INTO` snapshot | Snapshot into a new file | Existing SQLx/SQLite runtime; no external client |

PostgreSQL clients must be version 12 or newer. The dump and restore tools use
the same major version, and a restore target must not be older than the dump
producer. MySQL clients require 8.0 or newer; MariaDB clients require 10.5 or
newer. This first release requires matching MySQL/MariaDB major/minor series
and rejects downgrade restores.

Capabilities use the actual driver identity. CockroachDB and TiDB do not
inherit PostgreSQL or MySQL backup support.

PostgreSQL restores use portable ownership/privilege options and report that
choice in the plan. Roles and other cluster-global objects are outside this
database-scoped workflow. MySQL/MariaDB consistency depends on storage engines
and concurrent DDL; inspect the plan's warnings. SQLite snapshots use
`VACUUM INTO`, which can change implicit ROWIDs for tables without an explicit
`INTEGER PRIMARY KEY`.

MySQL/MariaDB restore validates native dump headers, completion markers, and
database-routing restrictions. Dumps containing references to other databases,
dynamic SQL, administrative/file commands, or unsupported SQL modes require
manual migration. This validation does not make untrusted SQL safe to execute.

Managed SSH forwarding is reused by native clients where its TLS semantics can
be preserved. PostgreSQL keeps the logical server name separately from the
forwarded address. Native MySQL/MariaDB certificate-verifying modes, and MariaDB
require-TLS mode, are currently rejected because their client trust settings
cannot be reproduced from the saved profile. Preflight explains the unsupported
combination instead of weakening the profile's TLS policy.

Full-database backups require the entire selected database to be visible in
the connection's catalog scope. Restricted schema selections are rejected
rather than silently exporting hidden schemas.

## TUI

Use `F2` to find:

- **Backup Database**
- **Restore Database**
- **Copy Database**
- **Data Tasks**

Select the intended connection/database before opening the form. Source and
destination bindings are captured explicitly; changing the active SQL console
does not redirect a running operation.

Enter the archive path and, for restore/copy, the new destination database or
SQLite file. Review the resolved plan, native tools, steps, and warnings before
execution. Existing destination databases/files and existing backup output
files are never overwritten.

Tasks run while the LazyDB process is open. The task view exposes progress,
results, cancellation, and rerun. Rerun opens a fresh draft: choose a new
archive path or destination where the previous task already created one.
An ordinary quit request opens Data Tasks while work is active. Wait for it
to finish, or explicitly cancel the task and wait for cancellation before
quitting again. Shutdown cleanup requests cancellation; tasks do not detach
into a daemon. Abrupt process termination is recorded as interrupted when
history is next loaded.

## CLI

Data commands use saved connections, resolved by name or UUID with the same
project visibility rules as the other headless interfaces. `--profile` is also
accepted as the default connection for backup and restore.

Commands preview by default. Add `--execute` to perform the reviewed operation.

```sh
lazydb data backup \
  --connection development \
  --output ./application.dump

lazydb data backup \
  --connection development \
  --output ./application.dump \
  --execute

lazydb data restore \
  --connection development \
  --database application_restored \
  --input ./application.dump \
  --execute
```

For SQLite, `--database` on restore is the new file path.

A complete copy keeps its intermediate native archive:

```sh
lazydb data copy \
  --source staging \
  --source-database application \
  --destination development \
  --destination-database application_copy \
  --output ./application-copy.dump \
  --execute
```

To review and execute exactly the same saved plan:

```sh
lazydb data backup \
  --connection development \
  --output ./application.dump \
  --save-plan ./backup-plan.json

lazydb data run ./backup-plan.json --execute
```

Plans bind the operation, connection configuration, resolved paths, native
tools, and input archive checksum where applicable. Execution rechecks those
bindings. Changed connections, archives, or tool versions require a fresh
plan. An executed plan ID cannot be reused.

Use `--dump-tool /path/to/pg_dump` and
`--restore-tool /path/to/pg_restore` to select native clients explicitly.
For MySQL/MariaDB the restore tool is their SQL client.

History and rerun:

```sh
lazydb data jobs --limit 20
lazydb data show TASK_UUID
lazydb data rerun TASK_UUID --output ./another-backup.dump --execute
lazydb data rerun RESTORE_TASK_UUID \
  --destination-database another_restored_database \
  --execute
```

`--json` returns a versioned envelope with `ok`, `command`, `data`, and `error`.
Progress stays out of JSON stdout. Execution exits with `0` on success,
`1` on failure, and `130` for a cancelled task.

Destination writes default to `--write-policy non-production`. Production
targets require `--write-policy all`. A read-only profile or global
`--read-only` always blocks destination writes. Backup and copy can read a
read-only source; copy requires a writable destination profile.

## Artifacts and Verification

The archive retains its database-native format and can be used without
LazyDB. Completed backups also have a `<archive>.lazydb.json` sidecar with
engine/tool information, scope, size, SHA-256, warnings, and verification
results. Credentials are not stored in task records, plans, or sidecars.

Native output is staged and published without replacing an existing file.
Cancellation or failure never promotes an incomplete archive to a successful
backup. The sidecar and task's verified backup outcome are persisted before
copy begins restoring. If the restore fails, the completed archive, checksum,
and backup verification remain available in task history. Sidecar publication
failure stops the operation before destination writes and reports the retained
archive for inspection.

Verification results name what actually ran. Archive readability, checksum
verification, successful restore execution, and SQLite integrity checks are
distinct from source/target data-parity verification. LazyDB does not claim a
full recovery rehearsal or exact data parity merely because a dump command
completed.

Restores validate available sidecar checksums before creating the destination.
A native archive without a LazyDB sidecar can still be used when its format
passes the provider's checks.

Restore pins the validated input in a private temporary copy before creating
the target. Allow temporary disk space for another copy of the archive as well
as the new database.

Restoring a native archive executes its database definitions and statements.
Only restore archives whose source you trust, using the intended destination
connection and database permissions.

## Task Lifecycle

Tasks record preflight, execution, verification, success, warnings, failure,
cancellation, or interruption. Progress shows actual stages and observed
amounts; an unknown total is not presented as a percentage.

Task records live in `data-jobs` beside the connection configuration. Each
process holds a task lock while running, so another process listing history
does not incorrectly mark a live task interrupted. Destination and artifact
locks prevent conflicting LazyDB tasks within that store.

Plan and task-record JSON files are limited to 4 MiB; backup sidecars are
limited to 1 MiB. Metadata readers require regular files and enforce the
limit on actual bytes read.

On failure or cancellation, the new destination may be partially populated.
The task reports this uncertainty and retains the target for inspection.
LazyDB does not automatically delete it or promise rollback across a complete
restore. Retry uses a new reviewed plan and a new target when necessary;
generic resume from an arbitrary committed batch is not supported.

## MCP

The MCP server exposes:

- `plan_data_operation`: read-only preflight and review.
- `list_data_tasks`: bounded task history.
- `get_data_task`: one task's status and verification.

These tools do not execute backups or restores. Destination write-policy
checks still apply during planning. Only visible connections and
project-contained archive/SQLite paths are accepted; relative paths resolve
against the project root. MCP cannot select arbitrary native executables.
Execute operations through the TUI or CLI.
