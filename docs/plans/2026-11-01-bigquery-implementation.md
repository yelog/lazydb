# BigQuery Implementation Plan

> **For Claude:** Execute the tasks below in this worktree. User authorization is to implement and verify now; do not commit, push, or pause for a plan handoff.

**Goal:** Add native Google BigQuery support through the existing relational TUI and agent workflows, with refreshable ADC, bounded results and native billing limits.

**Architecture:** Use reqwest against the fixed verified Google BigQuery REST endpoint and a maintained ADC library. Submit one GoogleSQL statement with a client-generated job ID, recover uncertain submissions by reading that ID, poll and page with project/location preserved, and cancel through the native API. Keep execution project distinct from catalog/data project; use existing immutable scope and target ownership boundaries.

**Tech Stack:** Rust 1.94, Tokio, reqwest, sqlparser BigQueryDialect, Google Cloud authentication, deterministic loopback HTTP fixtures.

---

### Task 1: Inspect and verify contracts

Read CONTRIBUTING.md, docs/architecture.md, profile/profile compatibility, Snowflake/ClickHouse transport/query/catalog/relation, SQL policies, query/value models, persistence and execution target/runtime cancellation. Consult official jobs.insert/get/getQueryResults/cancel, GoogleSQL lexical and ADC/Rust API documentation. Compare official google-cloud-auth with gcp_auth, checking actual dependency manifests and compiler requirements.

### Task 2: Profiles and product configuration

Modify src/profile.rs, src/profile_compatibility.rs, src/model/profile_manager.rs, src/ui/profiles.rs, config/default.toml and related tests. Append BigQuery kinds and formats without reordering shared enums. Add ADC path, data project, location, maximum_bytes_billed (1,000,000,000 default) and query_timeout_secs (120 default). Validate secret-free bigquery:// URLs, persistence and fingerprints. Hide password inputs for ADC.

### Task 3: Driver and authentication

Create src/db/bigquery/{mod,auth,transport,query}.rs. Fix production endpoint to https://bigquery.googleapis.com/bigquery/v2/. Load ADC using maintained library; never execute gcloud. Bound response and aggregate bytes, avoid response URL forwarding, redact credentials and auth failures. Implement stable jobs.insert identity, uncertainty recovery, bounded polling/paging, native cancellation and faithful value conversion. Add deterministic auth/HTTP fixture tests before live testing.

### Task 4: Catalog and relation operations

Create src/db/bigquery/{catalog,relation}.rs. Map projects/datasets/tables/views and nested fields through native paginated APIs. Intersect profile/request scopes, distinguish partial permissions, warn about incomplete projects.list discovery, preserve selected inaccessible-to-list projects. Preview with lookahead and cost limits; obtain native INFORMATION_SCHEMA DDL only. Do not default to COUNT(*).

### Task 5: SQL, dispatch and policy integration

Modify src/sql/**, src/agent/** and required shared enum dispatch sites. Route BigQueryDialect, quote identifiers correctly, retain GoogleSQL ARRAY/STRUCT/UNNEST semantics. Reject scripts/transactions and conservatively classify unknown statements. Integrate descriptor/capabilities, persistence, target validation and cancellation registry without changing App::update side-effect rules. Disable unsupported mutation/monitor/principal/grid actions.

### Task 6: Verification and documentation

Add deterministic tests for URL/form/persistence/targets, auth refresh/concurrency, native request semantics, async/empty/multiple pages, errors/recovery, budgets/types/catalog and policy. Add explicit opt-in ignored live tests with isolated project/dataset and small billing limits. Exercise real TUI via the existing PTY approach with isolated configuration. Add docs/bigquery.md, README/docs index/configuration/capability matrix and release notes.

Run targeted BigQuery and Snowflake/shared regressions, then:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Record actual outcomes and distinguish deterministic fixture, PTY and real-cloud verification. Inspect final diff and list shared files that may conflict with the independent OpenSearch worktree. Never claim unavailable live verification passed.

## OpenSearch integration rehearsal — 2026-11-01

A read-only three-way text merge against the common base found 37 overlapping
tracked files. Fourteen require textual conflict resolution:

- CHANGELOG.md and docs/configuration.md.
- src/app.rs, src/cli.rs, src/runtime.rs.
- src/db/descriptor.rs and src/db/mod.rs.
- src/model/cell_editor.rs and src/model/profile_manager.rs.
- src/persistence/profiles.rs and src/ui/profiles.rs.
- tests/database_capabilities.rs, tests/driver_categories.rs and tests/execution_target.rs.

The other 23 overlapping files merged textually in the rehearsal; this does not
prove semantic compatibility. Both worktrees remained unchanged. The list is a
snapshot and must be recomputed after either implementation changes.

When integration is authorized, preserve both independent driver variants and
profile options. The combined registry must contain 17 drivers, with 12 relational
and 5 non-relational entries. Resolve the registry, CLI driver list and exact-list
tests together. Keep OpenSearch on its REST/JSON path and BigQuery on GoogleSQL;
BigQuery's billing/data-project distinction and cooperative native cancellation
must survive edits to shared target and runtime branches. Older-profile conversion
literals need both optional configuration fields. Driver-specific form fields,
credential clearing and discovery fingerprints need both implementations.

After conflict resolution, run formatting, all-target/all-feature Clippy and
full tests, then both drivers' PTY checks. Use BigQuery's explicit cloud checklist
in tests/fixtures/bigquery/README.md for the separate cloud acceptance gate; missing
credentials or designated cloud resources leave that gate unverified.

## Real-cloud acceptance — 2026-11-02

User-authorized acceptance used an existing Google Cloud project in BigQuery
Sandbox, developer authorized_user ADC acquired through the official CLI, and a
unique US dataset. No billing upgrade, new project or IAM change was performed.
The CLI ran outside LazyDB solely for interactive login; production queries used
the official Rust SDK and fixed BigQuery REST endpoint.

The expanded ignored live test passed native types (including BIGNUMERIC, JSON,
nonempty bytes and a negative microsecond timestamp), empty-result schema,
1,005-row native pagination, statistics, columns, preview, native DDL and exact
UUID-table cleanup. The initial empty-result SQL lacked FROM, which the live
service rejects; the regression now uses a constant UNNEST source.

The production binary also passed cloud PTY acceptance for form/Test Connection,
Explorer, preview/DDL, SQL grid/history, restart and terminal restoration. Its
JSON CLI passed native value/statistics checks, the existing 500-row truncated
budget, native billed-byte rejection and US/EU location mismatch. One PTY
attempt selected the integration test's concurrent transient table; rerunning
after that test completed passed. Future cloud workflows should isolate their
datasets or run serially.

After inspecting the dataset's remaining objects, acceptance dropped only its
own table and then its own dataset without CASCADE and confirmed deletion. ADC
stayed outside the repository with mode 0600; credential contents were never
copied into profiles or output. Latest changes passed cargo fmt --check,
Clippy for the live-test target with all features, and the real-cloud test.
Earlier full-suite/all-target checks remain recorded separately.

Service-account login, restricted IAM, live expiry refresh and confirmed
termination of an active remote job remain unverified; see the fixture checklist
for the exact remaining matrix.

## Cross-project and reusable cloud TUI follow-up — 2026-11-02

The follow-up enabled BigQuery API in a second existing project after read-only
service/permission checks. It created a uniquely named disposable US dataset in
that data project while query jobs belonged to the original execution project.
The existing ignored integration test passed all native types, pagination,
catalog/preview/DDL and job-ownership assertions with these distinct projects.

Added scripts/ci/test-bigquery-live-tui.py with mandatory --run-live and all five
explicit live-test environment variables. It uses the production binary and
creates/drops only its own UUID table with one-hour expiry. Exact Explorer search
avoids row-order races with concurrent transient tables. The script passed real
cross-project form/Test Connection, search, preview/DDL, console grid/history,
existing-console target isolation after Explorer focus changes, restart,
terminal cleanup and native job project/location/billing-cap checks. Native
integration and TUI tests ran concurrently with independent fixture tables and
both completed exact-table cleanup. Missing opt-in/environment checks were
verified to fail before cloud access.

The OpenSearch source worktree no longer existed. Its available remote-tracking
branch at 5460c22 retained the full implementation and CI selector fixes. Combined
integration was therefore prepared in a new detached worktree from that revision
and a snapshot of the current uncommitted BigQuery implementation. The original
BigQuery worktree remains separate and receives no OpenSearch changes; shared
history remains unchanged. Final combined validation is recorded below.

Integration regressions exposed two semantic issues beyond textual conflicts:
OpenSearch's cancelled-result guard also rejected BigQuery's owned cancellation
completion event, and raw profile persistence could restore cleared transport
fields and credential references when switching drivers. The latter also
predated OpenSearch: a same-ID Postgres-to-BigQuery save/reload could make the ADC
profile unavailable. Its minimal fix and independent regression were backported
to the original BigQuery worktree; unknown extension fields remain preserved.
The combined tree additionally tests OpenSearch-to-BigQuery-to-OpenSearch
roundtrips. The live TUI script's exact-table cleanup was verified after an
intentional TUI-launch failure, in addition to the successful real-app path.

## Combined validation completed — 2026-11-02

The `BigQuery-OpenSearch-集成验证` worktree is preserved, detached at
5460c22 with all integration changes uncommitted. All 14 textual conflicts were
resolved, and the combined registry contains 17 drivers: 12 relational and five
non-relational. No commit, push, rebase, reset or shared-history change occurred.

Formatting, all-target/all-feature Clippy with warnings denied, and the
all-feature production build passed. The full all-target/all-feature suite
reported 3,793 passes, zero failures and 26 ignored tests across 210 test binaries.
All five local PTY workflows passed: BigQuery fixture/catalog/query and offline
profile/restart, OpenSearch HTTP and HTTPS consoles, and the OpenSearch encrypted
credential/CA/authentication profile lifecycle. The combined production binary
also passed the reusable real-cloud cross-project BigQuery TUI workflow.

Dedicated OpenSearch fixture containers and volumes were removed, leaving
pre-existing containers untouched. All acceptance-created cloud tables and
both disposable datasets were deleted; the cross-project dataset was checked
empty and deleted without deleteContents/CASCADE, then confirmed absent. The
two existing projects retain their enabled BigQuery APIs. ADC remains outside
the repository; no billing upgrade or IAM change was performed.

Non-sensitive logs and a cloud-validation manifest are retained outside the
repository. The reusable script and fixture README provide the repeatable
procedure. Service-account login, restricted IAM,
natural token-expiry refresh and confirmed active-job termination remain
unverified in the real cloud.

After backporting the persistence fix, the original BigQuery worktree passed
118 tests with zero failures/ignored across seven focused suites: BigQuery
profiles, persistence, profile compatibility, transactions, Elasticsearch auth
profiles, runtime and reducer. The new same-ID Postgres-to-BigQuery save/reload
regression passed. All-target/all-feature Clippy with warnings denied and the
all-feature production rebuild also passed. The shared target's production
binary now corresponds to this original BigQuery source again; the combined
production binary is retained separately alongside its validation logs.

## Natural token-expiry acceptance completed — 2026-11-02

The explicitly gated developer-ADC lifecycle test passed after 65 real minutes
(3,903.17 seconds total runtime). The same adapter and SDK provider remained alive
throughout; sensitive token values were compared only in memory and confirmed
to rotate. The final zero-scan query succeeded without reconnecting or simulating
time, retaining the configured execution project, location and billing cap.
Initial job: `lazydb_53619a068b8742cf865be956f61ad282`; final job:
`lazydb_7c59f554e8ef465396188889c172f9c9`.

This closes the natural-expiry gap recorded earlier. Service-account login,
restricted IAM and confirmed active-job termination remain unverified. See
[merge readiness](2026-11-02-bigquery-merge-readiness.md) for the subsequent
correctness fixes, final 3,803-pass combined suite, standalone checks and
cancellation acceptance limits. No commits or remote history changes occurred.
