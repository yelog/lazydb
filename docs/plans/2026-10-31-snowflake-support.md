# Snowflake Support Implementation Plan

> **For Claude:** Execute the following tasks in this worktree; implementation is already authorized by the user. Do not commit or push.

**Goal:** Add Snowflake connections, SQL execution, database/schema/table/view browsing, column metadata, previews, DDL, and SQL dialect support.

**Architecture:** Reuse the existing DatabaseConnection enum and capability gates. Implement Snowflake's HTTPS SQL API with the existing reqwest/rustls transport, bearer tokens held by the existing secret credential flow, asynchronous polling and partition-aware bounded result fetching. Preserve database/schema scope and fully qualified quoted identifiers; advertise only implemented capabilities.

**Tech Stack:** Rust, Tokio, reqwest, serde_json, sqlparser SnowflakeDialect, existing TUI/CLI/configuration and query accumulator.

---

### Task 1: Connection model and product integration

Modify `src/profile.rs`, `src/db/descriptor.rs`, connection form, CLI, theme, SQL/completion dispatch and exhaustive matches. Add `DatabaseKind::Snowflake`, a `snowflake` URL format and optional Snowflake options (warehouse, role and token type). Default to HTTPS port 443 and PAT authentication, accept OAuth and externally generated key-pair JWT tokens, reuse password storage for the token. Preserve URL options across form editing. Add regression tests for URL round trips, secret handling, driver registration and dialect mapping.

### Task 2: SQL API transport and results

Create `src/db/snowflake/mod.rs`, `transport.rs`, `query.rs`. Submit one statement per API request using a UUID request ID, immutable database/schema/warehouse/role context and explicitly formatted dates/times. Handle HTTP 202/429 polling, SQL failures, JSON row metadata, gzip partitions, typed values, nulls, precise decimals and shared query row/byte budgets. Bound HTTP bodies, network timeouts and polling duration. Do not replay mutating POSTs automatically or follow response-supplied URLs. Add local mock HTTP regression tests for headers/body, asynchronous execution, partitions, truncation and errors; no cloud credentials needed.

### Task 3: Catalog and relation access

Create `src/db/snowflake/catalog.rs` and `relation.rs`. Use SHOW DATABASES and database-qualified INFORMATION_SCHEMA queries. Implement database/schema discovery, tables/views, column metadata, lazy catalog pages and search. Validate profile identities, path shapes and the intersection of profile/request catalog scope before issuing SQL. Use keyset pagination and server-side row limits for metadata. Implement bounded relation previews with validated WHERE/ORDER BY clauses, lookahead pagination, optional exact counts, and GET_DDL for tables/views. Add identifier/scope/pagination regression tests.

### Task 4: Adapter dispatch and capability gates

Modify `src/db/mod.rs` and `src/db/capabilities.rs`. Route implemented calls to Snowflake. Disable structured editing, principal management, monitoring, manual cross-request transactions and cancellation, with explicit unsupported errors. SQL statements run independently; SQL API sessions are not treated as persistent console sessions.

### Task 5: Documentation and validation

Document Snowflake connection examples, PAT/OAuth/JWT setup and current scope in `docs/snowflake.md`, configuration, README and capability matrix. Run `cargo fmt --check`, scoped Snowflake tests, existing connection/dialect/capability tests, `cargo check --all-targets` and broader relevant tests as practical. Record real-server validation as unavailable unless credentials are provided; never print credential values. Review final diff for unrelated changes and unsupported capability claims.
