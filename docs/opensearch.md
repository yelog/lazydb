# OpenSearch

LazyDB connects to OpenSearch's native REST API using an independent **OpenSearch** driver. It shares Elasticsearch's REST console, index/mapping browser, document preview, result budgets and write policy. OpenSearch profiles retain their own product identity and authentication options; no SQL plugin is needed.

## Connect

Choose **Non-relational → OpenSearch** in the connection form:

```text
opensearch://localhost:9200
opensearchs://admin@search.example.com:9200
```

Both schemes default to port `9200`. `opensearch://` selects HTTP; `opensearchs://` selects HTTPS with certificate-chain and hostname verification. HTTPS is never silently downgraded. Use OpenSearch's schemes rather than Elasticsearch's schemes to retain the correct driver identity.

Authentication supports **Basic Auth** (username/password) and **None** (no Authorization header). Select None explicitly for a security-disabled local server. Basic is the default; credentials in an imported URL select Basic and are moved into the existing secret credential flow. Generated OpenSearch URLs omit both username and password. Local encrypted or system storage can recover Basic credentials after restart. Changing authentication clears the previous credential. API keys, AWS SigV4, IAM/browser login, client certificates, and reverse-proxy base paths are not implemented.

For private trust, select HTTPS and set **CA file (optional)** to a readable PEM certificate or bundle. These roots supplement normal system trust; certificate and hostname verification remain enabled. Invalid/missing files and CA files with HTTP are rejected. Changing the CA or authentication invalidates the existing connection. The saved `opensearch_options` table contains `auth = "basic"` or `"none"` and optional `ca_cert_path`; passwords stay in `credential_policy`. These are connection-profile fields, not application settings in `settings.toml`.

The optional Database field is a virtual namespace, defaulting to `default`. OpenSearch has no SQL database/schema hierarchy; the namespace does not restrict REST paths to an index. Unknown URL parameters are rejected. The common `readOnly=true` URL option remains available.

## Permissions and discovery

OpenSearch Security permissions differ from Elasticsearch privileges. The initial `GET /` probe requires `cluster:monitor/main`; document reads need index read actions, mapping inspection needs mapping/metadata actions, and `_cat/indices` discovery can need cluster health/state and index-monitor actions. A principal that can search one index may still be denied index discovery or cluster-health requests. Administrators should grant only the actions needed by the intended workflow and inspect server permission errors. Do not copy Elasticsearch `_security` role examples into OpenSearch: OpenSearch Security administration uses `/_plugins/_security/api`, and available roles/actions depend on plugin and deployment configuration. Managed services may restrict these APIs further.

Expand the connection, virtual namespace and **Indices** group. Hidden/system indices starting with `.` are excluded. Discovery uses `GET /_cat/indices?format=json&h=index&expand_wildcards=open`. Expand an index for nested properties and multifields such as `title.keyword`; open its Data preview to inspect documents. The grid preserves `_index`, `_id`, `_score`, complete `_source`, the original `_hit`, and top-level source fields without allowing source names to overwrite metadata. Grid cells show bounded previews. Press `v` to open the record inspector, select `_source` with `j`/`k`, and press Enter to open the complete JSON value; `h`/`l` or arrow keys scroll it, and Esc returns to the inspector.

Previews use `from`/`size` within a 10,000-result window. The server's `index.max_result_window` can impose a smaller bound. Pages are not a point-in-time snapshot and can change between requests. Last-page navigation beyond the client window is unsupported. Use native PIT/`search_after` requests manually for larger or consistent searches where the server supports them. SQL WHERE/ORDER BY preview controls are unavailable; use Query DSL in a console. Each preview page has a 16 MiB result budget. Exceeding it returns an explicit error rather than treating truncated output as a completed page. Reduce the page size, or query through the console with `_source` filtering to limit document payloads.

## REST console

Create a console bound to the OpenSearch connection and execute one complete request:

```http
POST /books/_search
{"size":25,"query":{"match":{"title":"rust"}},"aggs":{"authors":{"terms":{"field":"author.keyword"}}}}
```

The first line is `METHOD /path?query=value`, followed by optional JSON. Normal execution and full-buffer execution both send one native request. `_bulk` and `_msearch` accept NDJSON; LazyDB adds the required trailing newline:

```http
POST /books/_bulk?refresh=wait_for
{"index":{"_id":"one"}}
{"title":"Learning Rust"}
{"index":{"_id":"two"}}
{"title":"OpenSearch Guide"}
```

Multiple REST requests in one execution are rejected. Requests stay on the configured endpoint; external URLs and redirects are rejected. Traversal, encoded separators and residual percent escapes are rejected before URL normalization; identifiers containing literal percent signs are currently unavailable through this console. Encoded `?` and `#` within identifiers remain path content. Cat APIs require `format=json`. Search responses show document hits and a separate metadata result containing totals, aggregations and shard information; other responses remain JSON. Common row/byte budgets apply, with a 32 MiB HTTP response limit. Reduce `size` or narrow queries when bounded output is insufficient.

Known read APIs execute according to the confirmation policy. Writes, unknown APIs and stateful requests require confirmation, including document reads that request `refresh` and searches that request `scroll`. Read-only profiles reject these requests before dispatch, including if the profile becomes read-only while confirmation is open. Database grants remain authoritative. Console text is retained in local history, so do not put credentials into request bodies or parameters.

HTTP 200 can contain partial failure. Bulk item failures, failed/incomplete multi-searches, multi-get document errors, nonempty `failures`, search timeouts and shard failures show an error summary and record failed history while retaining bounded response details. Successful bulk items remain committed. Removing status/error fields with `filter_path` prevents reliable failure detection. Inspect returned items and server state before retrying.

A disconnect or timeout after a write is sent can leave its outcome unknown. A successful HTTP response that exceeds the response limit or cannot be decoded also leaves the write result unknown; the error and local history retain that uncertainty. There is no automatic retry or rollback. Cancellation drops the local HTTP future and stops waiting; it does not prove that OpenSearch stopped work or undid a write. Check document IDs, bulk items and server state before issuing another mutation.

## CLI and coding agents

The existing connection commands retain the OpenSearch driver identity:

```sh
lazydb connections add --name local-search --scope global --url 'opensearch://admin@localhost:9200' --password-stdin
lazydb connections list
lazydb connections show local-search
lazydb connections test local-search
lazydb agent query --connection local-search --sql 'GET /_cluster/health'
```

The first command reads the password from stdin; keep credentials out of argv and
shell history. For HTTPS/private CA, create or edit the profile through the form
to set verified HTTPS and its CA path. For security-disabled HTTP choose None
explicitly through the form.

The existing agent `--sql` field carries the complete native REST request for an
OpenSearch profile; it does not enable SQL execution. MCP uses the existing
`query` and `execute_change` tools with the same REST payload, profile read-only
checks, bounded responses and explicit write policy. Query rejects mutations;
use the change path with the required approval and database permissions.
OpenSearch SQL/PPL plugins, IAM/SigV4 and Serverless-specific APIs are not
supported integrations. See [coding-agent access](coding-agent-access.md).

## Release notes and boundaries

This release adds a distinct OpenSearch driver with Basic/None authentication, verified HTTPS and PEM CA bundles, index/mapping discovery, document previews, REST/NDJSON execution, confirmation and read-only enforcement. Existing Elasticsearch profiles keep their identity and options. No SQL emulation, manual transactions, grid-cell editing, generated SQL DDL, structured principal forms or dedicated monitoring dashboard is advertised; administrative operations use explicitly confirmed REST requests where permitted.

The fixture pins the official stable `opensearchproject/opensearch:2.19.4` image, whose server reports distribution `opensearch` and version `2.19.4`. Other OpenSearch versions, OpenSearch 3.x, Amazon OpenSearch Service/Serverless and deployment-specific plugins require separate compatibility testing. Elasticsearch validation does not establish OpenSearch compatibility. The checks below do not prove remote cancellation, publicly trusted HTTPS, consistent deep pagination or every Security permission combination.

## Development checks

Run deterministic profile, transport, policy, console and Elasticsearch regressions without Docker:

```sh
cargo test --locked --test opensearch_profile --test opensearch_adapter --test opensearch_console
cargo test --locked --test elasticsearch_profile --test elasticsearch_auth_profile --test elasticsearch_adapter --test elasticsearch_console --test elasticsearch_response
cargo test --locked --lib db::elasticsearch
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The full repository suite also needs its DuckDB 1.5.6 C API runtime; set
`LAZYDB_DUCKDB_LIB_DIR` to its directory as in the existing CI runtime setup.
Only configure external database test URLs for disposable fixtures you intend
to exercise.

Start this independent Compose project (about 4 GiB total container memory limit):

```sh
docker compose -p lazydb-opensearch-test -f docker-compose.opensearch.yml config --quiet
docker compose -p lazydb-opensearch-test -f docker-compose.opensearch.yml up -d --wait --wait-timeout 300
export LAZYDB_TEST_OPENSEARCH_URL='opensearch://127.0.0.1:19300'
export LAZYDB_TEST_OPENSEARCH_TLS_URL='opensearchs://admin:LazyDB-local-test-Only-2026!@localhost:19301'
os_test_cert_dir=$(mktemp -d)
docker compose -p lazydb-opensearch-test -f docker-compose.opensearch.yml cp opensearch-tls:/usr/share/opensearch/config/test-certs/ca.pem "$os_test_cert_dir/ca.pem"
export LAZYDB_TEST_OPENSEARCH_TLS_CERT="$os_test_cert_dir/ca.pem"
cargo test --locked --test opensearch_live -- --ignored --nocapture --test-threads=1
cargo build --locked --bin lazydb
python3 -I scripts/ci/test-opensearch-tui.py
python3 -I scripts/ci/test-opensearch-tui.py --url "$LAZYDB_TEST_OPENSEARCH_TLS_URL" --ca-cert "$LAZYDB_TEST_OPENSEARCH_TLS_CERT"
python3 -I scripts/ci/test-opensearch-profile-tui.py
```

Prerequisites are Docker Engine/Compose with sufficient memory, the Rust toolchain, and Python 3.9+ with the standard library on a POSIX host. No Python packages or tmux are required. The Alpine `3.22.2` certificate helper installs OpenSSL at startup, so initial fixture setup needs package-network access. On Linux, OpenSearch may require host `vm.max_map_count >= 262144`; follow the official Docker setup for your host. The fixture binds only loopback ports `19300`/`19301`, with separate project-scoped data and certificate volumes. Override `LAZYDB_TEST_OPENSEARCH_PORT`/`LAZYDB_TEST_OPENSEARCH_TLS_PORT` for conflicts. HTTPS uses a 30-day disposable CA and localhost-only certificate. The HTTPS node uses OpenSearch's demo Security configuration for transport/security initialization, with a custom HTTP certificate. These public test credentials and disabled disk watermarks are for disposable fixtures only. To change the admin password set `LAZYDB_TEST_OPENSEARCH_PASSWORD` before the initial startup and update the URL; existing data volumes retain security state.

The ignored live tests fail if required fixture variables are missing. They create randomly named resources and clean up only those resources, covering None/HTTP, index/mapping discovery and search, 23-document and last-page previews, metadata/highlights/aggregations, bulk and multi-search NDJSON including partial failure, conflicts, profile read-only denial, Basic/HTTPS with single/bundled CA, unknown CA and hostname rejection, bad passwords and OpenSearch Security permissions. The permissions test requires the fixture administrator and creates/deletes a temporary role and internal user through OpenSearch's own APIs; never run it against a shared cluster.

The console PTY script drives the actual binary through console creation, read/query history, write confirmation before dispatch, partial bulk/msearch failure, server-state verification and read-only denial. Basic credentials enter its child through `LAZYDB_PASSWORD`, with no secret in the child argv or saved profile metadata. The profile PTY script starts with an empty configuration, creates Basic/HTTPS through the real form, tests/saves local encrypted credentials, restarts without a password environment variable, browses mappings/documents, resizes the live preview to 100/120/160 columns, opens a nested 66 KiB document in the complete-value inspector and scrolls beyond the grid preview, clears/restores the CA to verify rejection/recovery, then switches through the form to None/HTTP and restarts to verify credential removal and unauthenticated reads. Both use temporary application configuration and verify credential masking and terminal restoration. `--diagnostics-dir` saves fixed phase/check labels only, excluding raw screens, profiles, encryption keys and API responses.

The independent `opensearch` job in [CI](../.github/workflows/ci.yml) runs on Ubuntu 24.04 for pull requests, pushes to `main`, and manual workflow runs. It builds with Rust 1.94.0, starts both pinned nodes under a run-specific Compose project, exports the public CA, and runs the three live tests, HTTP/HTTPS console PTY checks and profile-form PTY checks. Every long-running stage has a timeout, and fixture cleanup runs even after failure. Failure artifacts have a seven-day retention and allow only fixed sanitized check labels and container lifecycle status; raw terminal output, profiles, encryption keys, API responses and service logs are excluded. A hosted run remains unverified until the workflow executes on GitHub.

On 2026-11-01, both fixture nodes reported OpenSearch `2.19.4` and all three Rust live tests passed: None/HTTP catalog/search/mapping and 23-document previews, custom single/bundled CA with unknown-CA/hostname rejection, and Basic/OpenSearch Security permission enforcement. The console PTY checks passed against both None/HTTP and Basic/HTTPS in writable and read-only modes, including partial bulk/msearch failures, history and terminal restoration. CLI add/list/show/test retained the OpenSearch URL/profile identity; agent query completed `GET /_cluster/health` and rejected a DELETE request on the query path. A real stdio MCP initialization and `query` call also completed the health request and rejected DELETE under the query policy. The profile-form PTY passed all nine phases on macOS with Python 3.9: real Basic/HTTPS creation and test/save, encrypted restart/read, Explorer catalog search/mapping/document preview, CA removal/rejection/restoration, and switching to None/HTTP with username/credential removal and successful restart/read. Credential masking and clean terminal restoration passed throughout. Follow-up preview acceptance resized the real terminal to 100/120/160 columns and read nested fields in a 66 KiB document, including scrolling to text beyond the former 80-character detail-view limit. Initial attempts exposed the truncated detail view and PTY resize/input timing; the viewer now loads complete values and the harness drains resize redraws before navigation. Both the local debug driver and the standard CI profile command subsequently passed all nine phases. No hosted OpenSearch CI run is claimed.

The initial implementation checks passed on 2026-11-01: `cargo fmt --check`,
`cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo test --all-targets --all-features` (207 targets, 3,702 passed, 0 failed,
24 ignored). The full-suite run used
`LAZYDB_DUCKDB_LIB_DIR=/tmp/lazydb-duckdb-runtime-20261101` and removed an inherited
`LAZYDB_TEST_POSTGRES_URL`; unrelated external database live suites were not
configured. OpenSearch's three ignored live tests were separately executed and
passed. The final binary repeated both console PTY flows, all nine profile PTY
phases, CLI connection/query checks and a stdio MCP query/policy check. Earlier
attempts exposed missing DuckDB runtime configuration and exhausted local disk
space; after configuring the existing runtime and cleaning only this worktree's
rebuildable package artifacts, the complete final run succeeded. The independent
OpenSearch fixture was removed with its project volumes after validation.

Follow-up checks on 2026-11-02 passed after the safety and preview fixes: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` (207 targets, 3,709 passed, 0 failed, 24 ignored), using the same isolated DuckDB runtime setup. Workflow syntax and expressions passed actionlint 1.7.7 with a temporary configuration registering the existing `macos-15-intel` runner label; ShellCheck was unavailable and its integration was disabled. Compose configuration validation and both Python script syntax checks also passed. The independent review found and prompted fixes for refresh authorization, secret-free URL username preservation, read-only reconnection, unknown write-response outcomes, preview lookahead budgeting, unsupported DDL restoration and diagnostic text corruption. The residual-percent path restriction remains intentional, and broader test-fixture/credential-reset refactoring was deferred to keep this task scoped.

After the complete Rust suite, one live rerun failed to connect to both loopback endpoints while Docker was unresponsive: the HTTP probe timed out, `docker info` exceeded 20 seconds, and project-scoped cleanup exceeded 45 seconds. Docker subsequently recovered without a shared engine restart. The final rerun on 2026-11-02 passed all three Rust live tests, both HTTP/HTTPS console PTY flows in writable and read-only modes, and all nine profile PTY phases, including 100/120/160-column resizing and complete nested long-document scrolling. These final runs used the standard CI commands and the final local binary. The `lazydb-opensearch-followup` project's containers, network and three volumes were then removed successfully; label-filtered container and volume queries returned no remaining resources. The [first hosted CI run](https://github.com/yelog/lazydb/actions/runs/41362791832/job/125016552408) subsequently passed the complete OpenSearch Ubuntu job, including live tests, both console PTY flows, profile persistence/reconnect and fixture cleanup. Ubuntu Rust and database-adapter checks also passed. The same run exposed an Elasticsearch profile script that selected the last driver by position and two retired `macos-14` runners; follow-up CI changes select Elasticsearch by visible identity and migrate the ARM64 runners to `macos-15`.

Remove only this disposable project's containers and volumes after validation:

```sh
docker compose -p lazydb-opensearch-test -f docker-compose.opensearch.yml down -v
```
