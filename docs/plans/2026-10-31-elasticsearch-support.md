# Elasticsearch Support Implementation Plan

> **For Claude:** Execute the tasks below in this worktree. The user has authorized analysis and implementation; do not create commits or pause for a separate execution choice.

**Goal:** Add usable Elasticsearch connections, index and mapping discovery, document previews, and a native REST/JSON console with existing confirmation and credential handling.

**Architecture:** Add an independent Elasticsearch HTTP adapter using the existing reqwest dependency. Reuse the generic catalog and result grid, represent the cluster as one virtual database/schema, and expose indices as relations with native kind INDEX. Add an explicit Search interaction model so Elasticsearch never enters SQL or MongoDB execution paths.

**Tech Stack:** Rust, reqwest/rustls, serde_json, Tokio, existing ratatui reducer/runtime architecture.

---

### Task 1: Connection configuration

**Files:** `src/profile.rs`, `src/model/profile_manager.rs`, `src/db/descriptor.rs`, `src/ui/icons.rs`, `tests/elasticsearch_profile.rs`.

1. Register Elasticsearch as a non-relational driver.
2. Parse and compose `elasticsearch://` / `elasticsearchs://` URLs, port 9200, optional basic credentials, and a default virtual namespace.
3. Extend the existing connection form and secret storage flow without a new dependency.
4. Test URL round trips, TLS, IPv6, validation, and password redaction.

### Task 2: HTTP transport and console request contract

**Files:** `src/db/elasticsearch/mod.rs`, `tests/elasticsearch_adapter.rs`.

1. Implement bounded HTTP JSON responses, connection/request timeouts, verified TLS, no redirects, sanitized errors, and a root identity probe.
2. Parse one REST request (`METHOD /path` plus optional JSON or NDJSON body); reject external URLs and ambiguous input.
3. Classify reads conservatively for confirmation and read-only access policies.
4. Convert search hits and arbitrary JSON responses to QueryOutcome without losing nested documents or aggregations; enforce result budgets.
5. Exercise actual HTTP requests with a local mock server, including auth, error categories, response limits, and result conversion.

### Task 3: Index discovery and previews

**Files:** `src/db/elasticsearch/catalog.rs`, `src/db/elasticsearch/relation.rs`.

1. Discover user indices through JSON APIs; present the virtual namespace and index relations with stable catalog pagination.
2. Read mappings including nested fields and multifields.
3. Implement scoped catalog search and enforce profile/native identity boundaries.
4. Preview documents through `_search`, preserving JSON and respecting the 10,000-result window; reject unsupported SQL filters and sorting.

### Task 4: Runtime and UI integration

**Files:** `src/db/mod.rs`, `src/db/capabilities.rs`, affected exhaustive DatabaseKind matches, console execution and policy modules, editor language and UI hints.

1. Dispatch adapter calls and expose accurate capabilities, including unsupported transactions, principals, monitoring, DDL, and grid mutations.
2. Route native REST text around SQL statement parsing, diagnostics, formatting, and transaction analysis.
3. Reuse existing pending-execution confirmation for writes and reject writes on read-only profiles.
4. Preserve normal console history, result rendering, cancellation, and workspace lifecycle.
5. Test console execution, confirmation/read-only behavior, catalog activation, and existing driver regressions.

### Task 5: Documentation and verification

**Files:** `docs/elasticsearch.md`, `README.md`, relevant documentation tests.

1. Document configuration, native requests, authentication/TLS, mapping and document browsing, permissions, and current boundaries.
2. Run `cargo fmt --all -- --check`, `cargo check --tests`, targeted Elasticsearch and affected regression tests, then the project test suite when feasible.
3. Report exact checks and any environment-dependent live-server validation that could not run.

**Scope decisions:** No official Elasticsearch SDK is needed for the REST surface. No SQL emulation, automatic transaction support, or optimistic grid editing. The initial offset-based preview deliberately refuses requests beyond Elasticsearch's default result window; PIT/search_after browsing can be added separately when its cursor lifecycle is designed.
