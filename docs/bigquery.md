# Google BigQuery

LazyDB uses the native BigQuery REST API and GoogleSQL through the existing
relational Explorer, SQL console, JSON CLI and MCP query/catalog interfaces.
Production requests use certificate-verified HTTPS at
`https://bigquery.googleapis.com/bigquery/v2/`. The project in a profile URL is
an identifier, never an arbitrary API hostname. There is no production endpoint
override.

## Authentication

Create **Relational → BigQuery** and use Application Default Credentials (ADC).
There is no password or manually supplied access-token field.

For local development, log in outside LazyDB:

```sh
gcloud auth application-default login
```

Alternatively, point ADC at a service-account JSON file:

```sh
export GOOGLE_APPLICATION_CREDENTIALS=/absolute/path/service-account.json
```

The optional **ADC credentials file** profile field selects a specific
`authorized_user` or `service_account` JSON file instead of automatic discovery.
Its path is saved, but the JSON, refresh token, private key and access token are
never copied into profiles or exported URLs. File permissions remain the user's
responsibility. Explicit files are limited to 1 MiB and Google OAuth token
endpoints; errors omit credential contents.

Authentication uses the official `google-cloud-auth = 1.4.0` library, pinned to
the last release using reqwest 0.12 (the auth crate declares Rust 1.85; the
resolved lockfile and transitive dependencies are checked with the project's
Rust 1.94). Its credential providers own token acquisition, signing,
caching, concurrent refresh and expiry handling. Newer auth releases introduce
reqwest 0.13; the alternative `gcp_auth` default discovery chain includes a
`gcloud` command fallback. LazyDB never runs a background CLI to obtain tokens.

Automatic ADC discovery is delegated to the official SDK: environment-selected
credentials, the local developer ADC file and metadata-service credentials are
part of its discovery chain. The SDK also implements external-account and
impersonation providers. Deterministic tests verify authorized-user credential
parsing, token acquisition/expiry/refresh/concurrency and malformed service
account redaction. Workload identity, impersonation and metadata-service
credentials have **not** been verified in their deployment environments.
Developer `authorized_user` ADC obtained through the official Google Cloud CLI
was verified against a real BigQuery Sandbox project on 2026-11-02. A subsequent
65-real-minute test retained the same adapter and SDK provider, confirmed token
rotation only in memory and successfully queried again without reconnecting.
Live service-account authentication remains unverified.
Automatic ADC configuration is trusted deployment configuration; explicit-file
validation does not constrain all SDK-discovered external credential sources.
Changing a credential file's contents at the same path requires reconnecting.

## Projects, datasets and location

| Setting | Meaning |
| --- | --- |
| `database` / **Billing project** | Execution project: owns the job and billing; requires `bigquery.jobs.create`. |
| `bigquery_options.data_project` | Default data/catalog project; defaults to the execution project. |
| `default_schema` / **Dataset** | Default dataset for unqualified tables. |
| `bigquery_options.location` | Job location, such as `US`, `EU` or `europe-west2`; must agree with the datasets used. |

Fully qualified references such as `` `data-project.analytics.events` `` may
access other projects allowed by Google IAM. This does not change the billing
project. Explorer uses project → dataset → table/view → columns, including
nested field paths and native ARRAY/STRUCT types. Materialized views are also
listed when returned by the native API.

**Visible Objects** is an application metadata scope. It does not replace IAM
or restrict explicit SQL at the Google service. Selected project/dataset names
can be used even when list permissions cannot discover them. `projects.list`
only discovers a subset of accessible projects (project-level role and API
availability); discovery reports that limitation. An all-project search cannot
claim an exhaustive result over dataset-only grants: select explicit projects
for authoritative search scope. Permission failures and partial discovery are
reported separately from an empty directory.

Console targets retain the configured execution project and default data
project. A console cannot silently rebind its default dataset to another data
project by focusing an Explorer node; use a profile with that data project or a
fully qualified SQL path. Cross-project relation previews and DDL use the
relation's full native identity while billing the configured execution project.
Changing authentication source, location, projects, limits or visible scope
invalidates connection/discovery state. Query ownership remains independent of
Explorer focus.

## URL and saved configuration

```text
bigquery://billing-project/analytics?data_project=data-project&location=US&maximum_bytes_billed=1000000000&query_timeout_secs=120
```

Supported options are `data_project`, `location`, `credentials_file`,
`maximum_bytes_billed`, `query_timeout_secs` and the shared `readOnly`/`read_only`
flag. Percent-encode reserved characters in file paths. Userinfo, passwords,
ports, network endpoint options, unknown/duplicate options and TLS downgrades
are rejected. Project IDs use lowercase Google project-ID spelling (6–30
characters); project numbers and domain-scoped legacy IDs are not accepted.
The dataset path is optional. Saved profiles use `kind = "bigquery"`,
`url_format = "bigquery"`, `ssl_mode = "verify-full"`,
`credential_policy = { policy = "none" }` and an optional `bigquery_options`
table. Normal profile creation, editing, Test Connection, save and restart
restoration apply. Test Connection submits a constant `SELECT 1` in the billing
project, then discovers metadata; it does not persist or replace the active
session.

| Option | Default | Valid values |
| --- | --- | --- |
| `maximum_bytes_billed` | `1000000000` (1 GB) | Positive integer up to signed INT64 maximum. |
| `query_timeout_secs` | `120` | `1`–`86400` seconds. |
| `data_project` | Execution project | Google project ID. |
| `location` | Omitted; service inference | Nonempty location identifier. Set it explicitly for reliable job recovery. |
| `credentials_file` | Automatic ADC | Nonempty file path, no JSON contents. |

## Cost and result limits

Every submitted query sets `useLegacySql=false` and `maximumBytesBilled`.
The same billing limit applies to SQL, table/view previews and native DDL reads.
Google enforces the billed-bytes cap; an estimate above it rejects the query.
This is separate from the local `QueryBudget` row and retained-byte limits.
**Neither `LIMIT` nor `maxResults` is a scan-cost limit.** Preview pagination
may scan data again on each page; lower the billing cap for unfamiliar data.

Previews use lookahead and unknown totals. LazyDB does not automatically run
`COUNT(*)`; exact total/last-page requests are unsupported. Filters and ordering
are validated using GoogleSQL and identifiers are quoted before submission.

Result retrieval requests at most 1,000 rows per page, bounds each response to
16 MiB, total decoded HTTP result responses to 128 MiB and retained results to
at most 1,000,000 rows / 64 MiB (or the caller's smaller budget). It stops
requesting pages as soon as a row/byte budget is exhausted and marks the result
truncated. Oversized responses fail explicitly; narrow the projection for very
large individual values. Schema survives an empty result. Server statistics
include job identity/project/location, processed/billed bytes, slot milliseconds
and cache hit when supplied; result status and output show billing information.

## Execution, errors and cancellation

Each request accepts exactly one classified GoogleSQL statement. GoogleSQL
backticks, project-qualified paths, ARRAY/STRUCT, UNNEST, raw/triple-quoted
literals and GoogleSQL completion/highlighting use the native dialect. Generic
result-query wrappers are disabled where they would change value-table or
ordering semantics. DML/DDL follows the existing read-only and write-confirmation
policy; unknown syntax is never assumed read-only and unsupported scripts are
rejected before submission. Read-only checks apply before TUI confirmation, again
at dispatch against the current profile, and in the adapter. Changing the flag
retires the old connection; secret-free URLs retain the flag on roundtrip.

Jobs use a client-generated ID. After a lost/ambiguous submission response,
LazyDB reads that job ID and never blindly replays SQL, including writes. If
recovery fails, the error includes the original project/job/location for manual
inspection. Do not retry a write until its remote outcome is known. Safe metadata,
job-status and result-page GETs use bounded retries; SQL submission is never
replayed. Polling has a total deadline and bounded backoff; result page tokens do
not imply that an unfinished job is complete. Returned URLs are never followed or given bearer
credentials. HTTP errors, native job failures, authentication, permissions,
quota/backend errors and location errors retain distinct codes/categories.

Cancel requests use `jobs.cancel`. Local waiting stops, but an accepted cancel
request is **not confirmation that execution stopped**. LazyDB performs a bounded
status read and reports separately whether cancellation was accepted, whether
the remote job is confirmed stopped/completed, or whether its final state is
unknown. The job can still incur charges. The query timeout also requests
cancellation, and `jobTimeoutMs` asks the server to stop overdue work without
guaranteeing it. Dropping a running future requests best-effort cancellation.
Unknown submission, polling or cancellation outcomes retain `Unknown` certainty
in execution history. A definitive submission rejection keeps its original error
and does not trigger cancellation or an unknown-billing warning.

There are no cross-request SQL sessions, Legacy SQL, scripts/multiple statements,
manual transactions, grid writes, structured catalog mutation, dedicated
monitoring or user/role administration. Native DDL is read from
`INFORMATION_SCHEMA.TABLES.ddl` when supplied; missing/unsupported DDL is reported
rather than synthesized as a replayable definition.

## Types

NULL and empty values remain distinct. BOOL/INT64/FLOAT64, base64 BYTES,
DATE/TIME/DATETIME and UTC TIMESTAMP values map to the existing result model.
NUMERIC/BIGNUMERIC retain exact decimal strings. Timestamp decimal/scientific
epoch seconds use integer arithmetic, including pre-1970 fractions. JSON keeps
its JSON text. ARRAY/STRUCT keep their nested structure as JSON text in the
scalar grid, with original native type metadata; nested numeric strings remain
strings to avoid downstream precision loss. STRUCTs with anonymous or duplicate
field names use an ordered array of `{name, value}` members so no value is
overwritten. Unknown native scalar types retain their type and readable value.
Values that cannot be safely decoded fail
explicitly rather than silently losing precision.

## Permissions

The identity needs `bigquery.jobs.create` on the execution project (commonly
`roles/bigquery.jobUser`) plus metadata/data permissions on the relevant data
project/dataset (commonly dataset-level `roles/bigquery.dataViewer`). Listing
projects requires additional project visibility; it is not necessary for
explicit selected dataset access. Metadata needs dataset/table list/get access;
querying tables/views needs the corresponding data access, including authorized
view restrictions. DML/DDL additionally requires the permissions for that exact
operation. No automatic billing activation, project creation or IAM changes
are performed. Google IAM remains authoritative.

## Tests and verified scope

Deterministic tests use loopback HTTP servers, fake access tokens and temporary
credential JSON; they do not read the developer's credentials. The HTTP endpoint
constructor exists only under `cfg(test)` and is unavailable to production
profiles. Run:

```sh
cargo test --lib db::bigquery
cargo test --test bigquery_profile --test bigquery_sql --test sql_dialect_mapping --test sql_risk --test agent_policy
python3 -I scripts/ci/test-bigquery-profile-tui.py --binary target/debug/lazydb
python3 -I scripts/ci/test-bigquery-tui.py
```

The profile PTY script uses the actual TUI with isolated profile/workspace and
missing explicit ADC credentials to verify the form/save/restart path. The second
script drives the actual TUI through Test Connection, catalog discovery, preview,
native DDL and SQL execution/history against a loopback fixture. It launches an
explicitly ignored library-test child with a `cfg(test)`-only endpoint hook;
production binaries have no endpoint override. Both PTY workflows passed locally.
Separately, the production binary was exercised through a PTY against real
BigQuery on 2026-11-02: profile creation, Test Connection, Explorer discovery,
relation preview, native DDL, SQL execution/history, restart restoration and
terminal cleanup passed. This cloud run used the fixed production HTTPS endpoint
and developer ADC, with no fixture endpoint or fake credentials.

Repository-wide checks remain `cargo fmt --check`,
`cargo clippy --all-targets --all-features -- -D warnings` and
`cargo test --all-targets --all-features`. On the local macOS acceptance host,
large reducer tests need `RUST_MIN_STACK=16777216`. Builds used
`CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0` to limit disk
usage. DuckDB tests require its separately installed runtime through
`LAZYDB_DUCKDB_LIB_DIR`; see [DuckDB setup](duckdb.md). For an offline run, unset
optional database-test URLs such as `LAZYDB_TEST_POSTGRES_URL`; otherwise their
presence opts into tests against those external instances.

For repeatable cloud-backed TUI acceptance, configure all five explicit
`LAZYDB_TEST_BIGQUERY_*` variables from the fixture instructions and run:

```sh
python3 -I scripts/ci/test-bigquery-live-tui.py --run-live --binary target/debug/lazydb
```

The script requires an existing disposable dataset, creates one unique table
with a one-hour expiration and drops only that table after success or failure.
It selects the exact table through Explorer search, so concurrent tests' tables
cannot change the preview target. It verifies persisted ADC/project/limit
settings, native preview/DDL, SQL history, console target stability after Explorer
focus changes, restart/terminal restoration and execution-project job statistics.
It never creates projects/datasets, enables APIs, modifies IAM or upgrades billing.
Without `--run-live` and all five variables, it fails before cloud access.

Ignored live tests require an explicitly designated isolated execution project,
data project and existing dataset, with a small billed-bytes cap and unique
resource names. See [fixture and live-test instructions](../tests/fixtures/bigquery/README.md).
Real-cloud acceptance passed on 2026-11-02 using developer `authorized_user` ADC
and a disposable US dataset in an existing BigQuery Sandbox project. The initial
run used the same execution/data project; a follow-up used two distinct existing
projects and verified execution-project job ownership,
data-project catalog/preview/DDL and console target stability after Explorer
focus changes. The ignored integration test verified native
scalar/nested types, empty-result schema, all 1,005 ordered rows across the
1,000-row response-page boundary, job statistics, catalog columns, preview,
native DDL and exact UUID-table cleanup. Production TUI and JSON CLI acceptance
also passed; the CLI retained BIGNUMERIC precision and a pre-epoch microsecond
TIMESTAMP, enforced its 500-row budget with truncation, and reported native
billed-bytes-cap and location-mismatch failures. The test-owned table and dataset
were inspected and deleted without CASCADE after acceptance.

Four separately gated, ignored library tests now cover natural token expiry,
active-job cancellation, existing service-account credentials and restricted IAM;
see the fixture instructions for exact commands and prerequisites. The expiry
check keeps one adapter/provider alive for 65 real minutes and compares token
rotation only in memory. The active-job cancellation test passes only after a
native terminal `stopped` reason, not merely an accepted request.

The real-cloud developer-ADC expiry test passed after 65 real minutes: the token
rotated and the same connection executed another zero-scan query. Live
service-account authentication, restricted-IAM identities and confirmed
termination of an active remote job remain unverified. Metadata-service, workload-identity
and impersonation deployments also remain unverified. Sandbox acceptance does
not establish support for operations that Sandbox itself disallows.

## References

- [ADC discovery](https://cloud.google.com/docs/authentication/application-default-credentials)
- [Official Rust auth API](https://docs.rs/google-cloud-auth/1.4.0/google_cloud_auth/credentials/index.html)
- [jobs.insert](https://cloud.google.com/bigquery/docs/reference/rest/v2/jobs/insert), [Job configuration](https://cloud.google.com/bigquery/docs/reference/rest/v2/Job)
- [getQueryResults](https://cloud.google.com/bigquery/docs/reference/rest/v2/jobs/getQueryResults), [jobs.cancel](https://cloud.google.com/bigquery/docs/reference/rest/v2/jobs/cancel)
- [GoogleSQL lexical rules](https://cloud.google.com/bigquery/docs/reference/standard-sql/lexical)
- [Native INFORMATION_SCHEMA DDL](https://cloud.google.com/bigquery/docs/information-schema-tables)
