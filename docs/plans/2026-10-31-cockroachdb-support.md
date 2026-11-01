# CockroachDB Support Implementation Plan

> **For Claude:** Implement this plan task-by-task within the authorized first-release scope.

**Goal:** Add CockroachDB connections, SQL execution, catalog browsing/search, native DDL inspection, manual transactions, and access through existing MCP tools.

**Architecture:** Preserve CockroachDB identity with a distinct DatabaseKind and DatabaseConnection variant. Share PostgreSQL wire execution and decoding through SQLx, while implementing CockroachDB catalogs and transactions independently. Disable structured data/schema/principal mutation and monitoring until implemented.

**Tech Stack:** Rust, SQLx 0.9 PostgreSQL protocol, existing catalog/transaction models, CockroachDB Docker integration tests.

---

### Task 1: Connection identity and configuration

Files: src/profile.rs, src/model/profile_manager.rs, src/connections/cli.rs, tests/profile_url.rs, tests/connections_cli.rs.

- Add CockroachDb identity, cockroachdb URL format, port 26257 and database defaultdb.
- Accept PostgreSQL URL formats in CockroachDB context without changing driver identity.
- Add optional shared PostgreSQL TLS/options configuration; preserve it through parsing, URL formatting, profile editing and persistence.
- Add explicit CLI driver selection and tests for URL identity, TLS encoding and backward compatibility.

### Task 2: Wire execution and CockroachDB adapter

Files: src/db/pgwire/, src/db/cockroach/, src/db/postgres.rs, tests/cockroach_adapter.rs.

- Share PostgreSQL value decoding and execution with result budgets.
- Implement independent connection/probe without PostgreSQL version gates.
- Implement scoped lazy catalogs, search, stable object identity verification, columns/keys/index metadata.
- Implement filtered/sorted/paged previews without xmin and DDL using SHOW CREATE.
- Implement transaction backend, preserving SQLSTATE, without automatic transaction replay; close cancelled sessions safely.

### Task 3: Application integration

Files: src/db/mod.rs, src/db/capabilities.rs, src/db/descriptor.rs, src/model/execution_target.rs, src/sql/dialect.rs, src/app.rs, src/runtime.rs, src/ui/icons.rs, src/ui/profiles.rs.

- Register driver and dispatch supported operations to CockroachAdapter.
- Declare unsupported structured editing, principals and monitoring explicitly.
- Map the SQL dialect to PostgreSQL and update execution targets, database/schema discovery and transaction failure state.
- Add UI labels/icons and preserve old database behavior.
- Update existing ConnectionProfile literals for optional pg_options.

### Task 4: Database fixture, documentation and CI

Files: docker-compose.cockroachdb.yml, docker/cockroachdb/, .github/workflows/ci.yml, tests/support/mod.rs, README.md, docs/.

- Pin a CockroachDB version, create isolated fixture and document local connection/test commands.
- Add real integration coverage for identity, data types, catalog scope, stale IDs, DDL and transactions.
- Make database integration tests mandatory in CI rather than silently skipped.

### Task 5: Verification

- Run cargo fmt and cargo check, then targeted profile/driver/target tests.
- Start the CockroachDB fixture and run its integration tests with LAZYDB_REQUIRE_DATABASE_TESTS=1.
- Run PostgreSQL regression tests against a local isolated fixture where available.
- Run relevant SQL/transaction/MCP tests and report actual results and unverified limitations.

Follow-up review and delivery include focused commits, integration with the
latest main branch, a pull request, and remote CI validation.
