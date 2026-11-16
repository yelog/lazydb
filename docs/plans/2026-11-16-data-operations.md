# Native Data Operations Implementation Plan

**Goal:** Deliver the first release of native database backup, restore into a new target, same-engine copy, and repeatable task history through the TUI and CLI.

**Architecture:** A shared data-operations service owns immutable plans, policy checks, credentials, durable job records, and cancellation. Native providers own database-specific preflight, execution, and verification. TUI and CLI use the same service; existing query previews are never used as backup input.

**Tech Stack:** Rust, Tokio, SQLx, existing credential and SSH infrastructure, native PostgreSQL/MySQL/MariaDB clients, SQLite online snapshot facilities.

## Scope

- PostgreSQL custom archives, MySQL/MariaDB SQL dumps, and SQLite snapshots.
- Full database backups, restore to a new database/file, and same-engine copies.
- No overwrite or drop of existing targets or output files.
- Explicit verification levels; command completion does not imply data-parity verification.
- Durable task history, immutable review, cancellation, interruption detection, and rerun with a fresh plan.
- Streamed table migration, heterogeneous schema conversion, incremental backups, and CDC remain later phases of the accepted design.

## Tasks

1. Define the shared operation, endpoint, plan, progress, result, and job contracts in `src/data_ops/types.rs`.
2. Implement native providers under `src/data_ops/native/`, with safe process arguments, private credential files, version checks, bounded diagnostics, cancellation, and exclusive artifact publication.
3. Implement durable job storage and the shared service under `src/data_ops/`, including policy checks, profile binding, artifact checksums, and job/resource locks.
4. Add `lazydb data` commands for plan/review, backup, restore, copy, task listing, inspection, and rerun. Keep JSON errors machine readable and support explicit tool paths.
5. Add contextual Omni commands, the data-operation dialog, a task-history view, and runtime event integration. Reuse existing profile and session credentials; keep reducer I/O-free.
6. Add policy-preserving MCP planning and task inspection where supported without introducing unrestricted filesystem or database writes.
7. Exercise SQLite round trips and failure cases, native-process fixtures, CLI parsing, TUI reducers, and available isolated PostgreSQL/MySQL/MariaDB fixtures.
8. Update capability and user documentation and run formatting, focused tests, and compilation.

## Acceptance

- Exported data is independent of the query preview limit.
- Existing databases and files are never overwritten.
- A running task cannot be redirected by changing UI selection or editing a profile.
- Cancellation and process failure never produce a successful job or an apparently complete partial artifact.
- History contains no credentials, and another live process's task is not marked interrupted.
- Restores report their actual verification level and partial-target possibility.
- Tests use temporary files and dedicated test databases only; user connections are never used for acceptance.

## Progress

- [x] Repository and architecture review.
- [x] Shared contracts and durable service.
- [x] Native providers.
- [x] CLI and agent access.
- [x] TUI workflow.
- [x] Acceptance tests and documentation.

## Validation Results

- Formatting and all-target, all-feature Clippy with warnings denied passed.
- Library regression: 1,413 passed, 6 ignored; existing MCP regression: 5 passed.
  These checks used an existing compatible DuckDB 1.5.6 runtime through
  `LAZYDB_DUCKDB_LIB_DIR`.
- Data-operation and MCP-protocol integration targets: 65 passed. Coverage
  includes immutable plans, cancellation, read-only source copies, bounded
  metadata reads, FIFO rejection, exclusive publication, and preserving a
  completed backup's checksum when restoration fails.
- Existing command, Omni, keyboard, mouse, help, documentation, CLI, connection,
  and write-policy regression targets passed.
- Isolated PostgreSQL 16, MySQL 8.4, and MariaDB 11.4 acceptance: 3 passed.
  The fixtures checked 15 jobs, including 3 deliberately failed copies with
  retained manifests and recoverable archives, 9 verified new databases, and
  9 overwrite refusals.
- Real SQLite PTY acceptance: 10 checks passed, covering F2 command selection,
  draft, review, execution, task history, archive contents, checksums, and clean
  terminal exit.
- Acceptance used temporary files and owned disposable databases. The fixture
  runner removed its containers and temporary root after verification.
