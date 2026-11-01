# OpenSearch Native Support Implementation Plan

> **For Claude:** Execute the tasks below in this worktree; the user's explicit request authorizes implementation and prohibits commits or history operations.

**Goal:** Add independently identified OpenSearch connections with complete native REST browsing, execution, policy, persistence, and acceptance coverage.

**Architecture:** Retain the Elasticsearch REST adapter as an internal shared transport with an explicit product kind. Add independent OpenSearch profile options, URL schemes, driver registration and connection dispatch; reuse bounded results and existing owner/generation enforcement.

**Tech Stack:** Rust, reqwest/rustls, Tokio, Ratatui, serde, Docker Compose, Python standard-library PTY tests.

---

### Task 1: Profile and connection form

Files: `src/profile.rs`, `src/model/profile_manager.rs`, `src/ui/profiles.rs`, `src/persistence/profiles.rs`, `tests/opensearch_profile.rs`.

1. Add OpenSearch kind, URL formats, Basic/None authentication and PEM CA options with backwards-compatible defaults.
2. Cover URL import/export, unknown/duplicate parameters, escaping, IPv6, secret extraction and persistence round trips.
3. Implement form creation/edit/test fingerprint/save/restart support and credential clearing when authentication changes.
4. Keep existing Elasticsearch fields and API Key behavior intact.

### Task 2: Shared REST adapter

Files: `src/db/elasticsearch/mod.rs`, `catalog.rs`, `relation.rs`, `tests/opensearch_adapter.rs`, Elasticsearch regression tests.

1. Capture product kind and select only that product's options. Probe OpenSearch `version.distribution` and nonempty version; reject cross-product identity.
2. Preserve verified TLS, no redirects, endpoint confinement, bounded HTTP responses and native NDJSON.
3. Reject traversal before request dispatch. Preserve conservative read classification and local cancellation.
4. Report uncertain write network outcomes without retry. Preserve metadata, nested JSON, partial-failure reporting and shared row/byte budgets.
5. Test actual local HTTP requests for identity/auth/permissions, browsing, pagination/window boundaries, unsafe paths, partial responses and response size limits.

### Task 3: Application dispatch

Files: `src/db/mod.rs`, `src/db/descriptor.rs`, `src/db/capabilities.rs`, `src/app.rs`, `src/runtime.rs`, `src/agent/policy.rs`, `src/cli.rs`, relevant model/UI/dialect files and tests.

1. Register independent product/connection variant/icon and native REST capabilities.
2. Route TUI and CLI/agent execution through existing REST confirmation and readonly policies.
3. Preserve immutable confirmation snapshots, current profile revalidation and asynchronous owner checks.
4. Disable unsupported SQL transactions/editing/formatting and return explicit unsupported results.

### Task 4: Isolated fixture and user documentation

Files: `docker-compose.opensearch.yml`, `tests/opensearch_live.rs`, `scripts/ci/test-opensearch*.py`, `docs/opensearch.md`, README and documentation indexes/matrix/configuration.

1. Pin an official stable OpenSearch version; bind only loopback with independent ports/project/volumes and disposable public credentials.
2. Exercise Basic/unauthenticated HTTP, verified HTTPS/custom CA/wrong hostname, restricted grants, mapping, search, NDJSON and preview pagination.
3. Drive actual form create/test/save/restart and explorer/console/write confirmation/readonly via PTY.
4. Document grants, URL/options, REST examples, window limits, uncertain writes, cancellation and precisely tested versions; add release notes according to repository convention.

### Task 5: Acceptance

1. Run `cargo test --test opensearch_profile --test opensearch_adapter --test opensearch_console` and relevant Elasticsearch regression targets.
2. Run fixture-dependent ignored tests and Python PTY scripts with the isolated fixture URLs and CA.
3. Run `cargo fmt --check`.
4. Run `cargo clippy --all-targets --all-features -- -D warnings`.
5. Run `cargo test --all-targets --all-features`.
6. Record exact outcomes and unrun scenarios in `docs/opensearch.md`; clean up only this task's fixture. Report shared-file conflicts relevant to the parallel BigQuery worktree.
