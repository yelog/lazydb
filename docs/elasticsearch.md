# Elasticsearch

LazyDB connects to Elasticsearch through its native REST API. It provides index discovery, mapping fields, document previews, and a REST console for Query DSL and management requests. It reuses the existing credential store and result grid; no Elasticsearch SQL plugin is required.

## Connect

Select **Elasticsearch** in the non-relational driver category, or import a URL:

```text
elasticsearch://localhost:9200
elasticsearchs://elastic@search.example.com:9200
```

`elasticsearch://` uses HTTP. `elasticsearchs://` uses HTTPS with certificate and hostname verification. Both default to port 9200.

Choose **Basic Auth** or **API Key** in the connection form. Basic Auth uses the username and password fields. API Key hides the username and accepts the **encoded** value returned by Elasticsearch's create API key response; paste that value directly, without an `ApiKey ` prefix or another base64 encoding. Both credentials use the existing local encrypted or system credential storage and are omitted from generated URLs. Switching authentication clears the previous credential, so enter the credential for the new mode. URL credentials always select Basic Auth; API keys are configured through the form rather than URL parameters.

For a private certificate authority or self-signed test certificate, select HTTPS and set **CA file (optional)** to a readable PEM certificate or PEM bundle. The adapter adds those certificates to its default trusted roots and still verifies the server hostname. Missing or invalid files produce configuration errors; HTTP with a CA file is rejected. Changing authentication or the CA path invalidates the existing connection. Client certificates, Cloud ID, and reverse-proxy base paths are not yet exposed.

The connection probe (`GET /`) requires the cluster `monitor` privilege. Document reads need index `read`; mapping inspection needs `view_index_metadata`; index discovery also needs index `monitor`. An account with cluster `monitor`, index `read`, and `view_index_metadata` can use the REST console but receives a permission error when discovering indices without index `monitor`.

The optional Database field names a virtual catalog namespace, defaulting to `default`. Elasticsearch itself has no database/schema hierarchy; this namespace helps LazyDB reuse its existing navigation and never restricts REST requests to an index. Unknown URL query parameters are rejected. `readOnly=true` remains available as the common profile setting.

## Browse indices and documents

Expand the connection and virtual namespace to find the index group. The group is labelled **Indices**. Internally the shared catalog represents indices as relations with native kind `INDEX`, avoiding changes to persisted catalog identifiers. Hidden/system indices whose names start with `.` are excluded from discovery. The discovery API is `GET /_cat/indices?format=json&h=index&expand_wildcards=open` and may require index-monitor permissions in addition to document-read access.

Expand an index to inspect its mapping fields, including nested properties and text multifields such as `title.keyword`. Open an index to preview documents. The grid retains `_index`, `_id`, `_score`, the full `_source` JSON, the original hit in `_hit` (including highlights, sort values, and inner hits), and separate top-level source fields. Nested objects and arrays remain JSON values; source fields cannot overwrite document metadata.

Previews use `from`/`size` pagination within Elasticsearch's default 10,000-result window. Results can change between pages because the preview is not a point-in-time snapshot. Last-page navigation beyond the window returns an explicit unsupported error. Use PIT and `search_after` manually in the REST console for larger or consistent searches. Preview filtering and sorting use the console's Query DSL; SQL WHERE/ORDER BY filters are disabled. Inexact server totals remain lower bounds at the result-window boundary. Preview pages obey the shared 16 MiB result budget; exceeding it returns an explicit error, so reduce the page size or use `_source` filtering in the console.

## REST console

Create a console and select the Elasticsearch connection. Both normal execution and full-buffer execution send one complete native request:

```http
POST /books/_search
{
  "size": 25,
  "query": {
    "match": { "title": "rust" }
  },
  "aggs": {
    "by_author": { "terms": { "field": "author.keyword" } }
  }
}
```

The first line is `METHOD /path?query=value`; following lines contain an optional JSON body. `_bulk` and `_msearch` accept NDJSON and receive the required trailing newline. Multiple REST requests in one execution are rejected. Paths stay on the configured endpoint; external URLs and redirects are rejected. Cat APIs must specify `format=json`.

Search responses show documents in one result set and response metadata (including total hits, aggregations, and shard failures) in another. Other API responses are retained as JSON. Results obey the common row/byte budgets, and HTTP responses are limited to 32 MiB. Narrow the query or reduce `size` when this limit is exceeded.

HTTP 200 does not mean every bulk operation, document fetch, or search succeeded. When `_bulk` reports failed items, `_msearch` contains failed/incomplete searches, `_mget` contains document errors, or an operation reports a nonempty `failures` array, the console shows an error summary and history records **failed** while retaining the returned JSON within the result budget. Search timeouts and shard failures are also marked as incomplete. Successful bulk items remain committed; inspect `items` before retrying, since LazyDB does not automatically retry or roll back the batch. Failure summaries remain available even when the result budget truncates the JSON. Agent results expose the same summary as `response_error` when present. These checks depend on the returned status/error fields; filtering those fields out with `filter_path` prevents reliable failure detection.

Examples:

```http
GET /_cluster/health
```

```http
GET /books/_mapping
```

```http
PUT /books/_doc/example?refresh=wait_for
{"title":"Learning Rust","author":"Example"}
```

Known read APIs can execute directly according to the confirmation policy. HEAD is restricted to known existence checks; encoded/traversal paths and unknown or stateful APIs require confirmation. Writes and unknown APIs require confirmation; read-only profiles reject them before sending, including when the profile becomes read-only while the confirmation dialog is open. Server privileges remain authoritative. Console commands are retained in local execution history, so avoid putting credentials or secrets in request bodies or query parameters.

## Release notes

This release adds native Elasticsearch connections with Basic Auth or API Key authentication, verified HTTPS and optional PEM CA bundles. Explorer discovers visible indices and mapping fields, previews documents, and opens a native REST/JSON console. Writes require confirmation, read-only profiles block mutations, and bulk/multi-search failures retain response details while recording failed execution history. Existing connection profiles remain readable; API keys use the existing encrypted or system credential store.

The first release does not provide SQL emulation, manual transactions, grid-cell editing, dedicated monitoring/principal forms, Cloud ID, or client certificates. Deep pagination uses manual REST requests rather than automatic PIT/search-after management. Compatibility evidence is limited to the exact versions and scenarios listed below; OpenSearch and Elasticsearch 9.x are not covered by that evidence.

## Current boundaries

- No SQL emulation, SQL formatting/completion, manual transactions, grid-cell editing, or generated SQL DDL.
- Index creation, document changes, aliases, settings, and other administrative operations use explicitly confirmed REST requests.
- No dedicated monitoring dashboard or principal-management forms.
- Cancellation drops the HTTP future and stops waiting locally; it does not guarantee cancellation of work already accepted by Elasticsearch.
- The adapter uses version-neutral REST endpoints. Elasticsearch 8.17.0 and 8.19.6 have been tested with the Docker fixture below; other versions still need compatibility testing.

## Development checks

Run deterministic transport, profile, policy, and console tests without a running Elasticsearch server:

```sh
cargo test --test elasticsearch_profile --test elasticsearch_auth_profile --test elasticsearch_adapter --test elasticsearch_console --test elasticsearch_response
cargo test --lib db::elasticsearch
```

The transport tests use local HTTP fixtures and check actual request methods, paths, authentication, request bodies, native pagination, partial search failures, redirect rejection, and the 32 MiB success / 16 KiB error response boundaries.

### Real Elasticsearch and TUI checks

Start isolated Elasticsearch 8.17.0 HTTP and HTTPS nodes (about 4 GiB total container memory limit):

```sh
docker compose -p lazydb-es-test -f docker-compose.elasticsearch.yml up -d --wait --wait-timeout 240
export LAZYDB_TEST_ELASTICSEARCH_URL='elasticsearch://elastic:lazydb-local-test-only@127.0.0.1:19200'
export LAZYDB_TEST_ELASTICSEARCH_TLS_URL='elasticsearchs://elastic:lazydb-local-test-only@localhost:19201'
es_test_cert_dir=$(mktemp -d)
docker compose -p lazydb-es-test -f docker-compose.elasticsearch.yml cp elasticsearch-tls:/usr/share/elasticsearch/config/test-certs/localhost/localhost.crt "$es_test_cert_dir/localhost.crt"
export LAZYDB_TEST_ELASTICSEARCH_TLS_CERT="$es_test_cert_dir/localhost.crt"
cargo test --test elasticsearch_live -- --ignored --nocapture
cargo build --bin lazydb
python3 -I scripts/ci/test-elasticsearch-tui.py
python3 -I scripts/ci/test-elasticsearch-tui.py --url "$LAZYDB_TEST_ELASTICSEARCH_TLS_URL" --ca-cert "$LAZYDB_TEST_ELASTICSEARCH_TLS_CERT" --api-key
python3 -I scripts/ci/test-elasticsearch-profile-tui.py --binary target/debug/lazydb
```

The fixture binds only to loopback. Its default password is public, disposable test data; never use it for a deployed cluster. Set `LAZYDB_TEST_ELASTICSEARCH_PASSWORD` and update the URLs if needed. Override `LAZYDB_TEST_ELASTICSEARCH_PORT` / `LAZYDB_TEST_ELASTICSEARCH_TLS_PORT` when those ports are occupied. Set `LAZYDB_TEST_ELASTICSEARCH_VERSION` to an exact image version to repeat compatibility checks; the default remains `8.17.0`. Use a separate Compose project and fresh volumes for each version, and keep that variable set for subsequent Compose commands.

The ignored Rust tests require an **isolated test cluster** and an administrator account: they create randomly named indices, users, roles, and API keys, then remove or invalidate only those resources. They cover native index/mapping discovery, 23-document pagination and last-page navigation, search highlighting and aggregations, bulk/msearch NDJSON, conflicts, authentication failures, server permission failures, API key invalidation, and profile read-only enforcement. HTTPS tests verify successful adapter connections with a custom certificate or PEM bundle, rejection without that trust, and rejection of an IP address absent from the certificate's hostname list. The fixture certificate covers only `localhost`; use that hostname for successful HTTPS connections. The disposable fixture disables Elasticsearch disk allocation thresholds to work in Docker VMs with little shared free space.

The POSIX PTY smoke script uses a temporary saved LazyDB profile, drives the actual binary through console creation, REST execution, confirmation before a write, and read-only rejection. With `--api-key`, it creates an expiring, index-scoped test key and invalidates it after use; `--ca-cert` enables the HTTPS fixture's explicit trust. Credentials reach the child through `LAZYDB_PASSWORD`, with no credential in its argv or profile metadata. The script verifies REST history, successful and partially failed bulk writes, partially failed multi-searches, the resulting server documents, credential masking, and terminal restoration on exit. It does not require tmux or Python packages.

The profile-form PTY script starts with an empty configuration and creates the API Key / HTTPS connection using the actual form, including connection testing and local encrypted storage. It restarts without `LAZYDB_PASSWORD` to verify credential recovery, switches to Basic Auth with a new credential, and clears/restores the CA to verify rejection and recovery. It requires `LAZYDB_TEST_ELASTICSEARCH_TLS_URL` and `LAZYDB_TEST_ELASTICSEARCH_TLS_CERT`, uses only temporary configuration, and invalidates its expiring API key on exit. Both scripts accept `--diagnostics-dir`: the console script retains a redacted terminal tail on failure, while the profile script saves only fixed phase/check labels and cleanup status, without screen contents.

### Continuous integration

The existing PR Rust job runs the deterministic Elasticsearch tests as part of `cargo test --all-targets --all-features`. A separate **Elasticsearch REST, TLS and terminal workflows** job runs on pull requests, pushes to `main`, and manual CI dispatch. It builds before starting the fixture, then runs all ignored live tests, both console authentication flows, and the profile-form acceptance script. Each command has a timeout; missing fixture configuration fails the job. The Compose project name is unique to the run and attempt, and cleanup runs regardless of earlier step failures.

On failure, CI retains bounded service logs and redacted terminal diagnostics for seven days. Configuration files, encrypted credential stores, local encryption keys, raw terminal transcripts, and raw API responses are excluded from the upload. The local commands above exercise the same acceptance paths; hosted GitHub execution occurs after the changes are pushed.

After testing, remove only this Compose project's disposable containers and certificate volume:

```sh
docker compose -p lazydb-es-test -f docker-compose.elasticsearch.yml down -v
```

Live validation on 2026-11-01 passed against Elasticsearch 8.17.0 and 8.19.6 with administrator and restricted Basic Auth accounts, plus API keys over HTTPS with a custom CA. Both versions passed the three live tests and the API Key/HTTPS console and profile-form acceptance scripts. Both Basic Auth over HTTP and API Key over HTTPS passed the real TUI checks in writable and read-only modes. The profile-form acceptance also passed on macOS with Python 3.9, including encrypted credential recovery after restart, authentication switching, and CA removal/recovery. Hosted Linux validation runs in the dedicated GitHub CI job; consult the pull request checks for the current revision's result. Other Elasticsearch versions, successful adapter HTTPS connections using publicly trusted certificates, cancellation of long-running requests, and PIT/search-after pagination are outside this validation.
