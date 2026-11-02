# BigQuery live fixture

The ignored `bigquery_live` test uses an **existing disposable dataset**. It does not create projects or datasets, change IAM, enable APIs, or change billing configuration. Running it submits billable BigQuery jobs; each job inherits a 10 MiB `maximumBytesBilled` cap and a 60-second timeout.

Provide all five variables explicitly. The test never reads development defaults or chooses an ADC file automatically. The credentials file can contain a supported Application Default Credentials configuration and must be an absolute path; keep it outside the repository. Do not paste credentials into commands, output, or commits.

```sh
export LAZYDB_TEST_BIGQUERY_PROJECT='your-billing-project'
export LAZYDB_TEST_BIGQUERY_DATA_PROJECT='your-data-project'
export LAZYDB_TEST_BIGQUERY_DATASET='lazydb_disposable_tests'
export LAZYDB_TEST_BIGQUERY_LOCATION='US'
export LAZYDB_TEST_BIGQUERY_CREDENTIALS_FILE='/absolute/path/to/protected/adc.json'
cargo test --test bigquery_live -- --ignored --exact bigquery_live_types_catalog_preview_and_native_ddl
```

The identity must already have permission to create query jobs in the billing project and create/read/drop its own tables plus read metadata in the selected dataset. Both projects may be the same. Use separate projects to validate that query job statistics retain the billing project while qualified table paths and metadata use the data project. The dataset location must match the explicit location.

The test creates exactly one `lazydb_test_<UUID>` table with one synthetic row and a one-hour expiration. It validates scalar types (including exact BIGNUMERIC, JSON, nonempty bytes and a pre-epoch microsecond timestamp), NULL versus empty text and bytes, lossless nested ARRAY/STRUCT JSON text, empty-result schema, 1,005 ordered rows across the 1,000-row response-page boundary, job billing statistics, relation children, preview, and native DDL. It drops only that exact UUID table after success or an ordinary returned failure. Expiration provides fallback cleanup after process interruption; verify the dataset if the test process is forcibly killed. Never use a production dataset or grant additional privileges merely to make this test pass.

No live fixture is run by the default test suite. Selecting `--ignored` is an explicit opt-in and all required environment variables must be present before any connection is opened.

## Cloud acceptance checklist

A cloud acceptance run on **2026-11-02** used developer `authorized_user` ADC
from the official Google Cloud CLI, one existing BigQuery Sandbox project for
both execution/data and a unique disposable US dataset. It passed the expanded
ignored integration test, production TUI form/Test Connection/catalog/preview/
DDL/query/history/restart/terminal cleanup, and production CLI native values,
job statistics, 500-row truncation, billed-byte-cap rejection and US/EU location
mismatch. Test-owned tables and the dataset were inspected and removed without
CASCADE. No billing upgrade, new cloud project or IAM change was needed.

The initial live run exposed invalid test SQL (`SELECT ... WHERE FALSE` without
FROM), which BigQuery rejects. The empty-result test now uses a constant UNNEST
source and passed against Google Cloud. Local fixtures had not exposed that
native SQL restriction.

A follow-up on the same date passed the native integration test and the
production TUI script with **distinct execution/data projects**. Job statistics
retained the execution project while metadata, preview and DDL addressed the data
project. Two successful executions in an existing console retained the execution
project and default dataset after Explorer focus changed. The live TUI used exact
unique-table search while the integration test created its own transient table;
both cleaned up their exact objects.

The developer-ADC expiry test subsequently passed in 3,903.17 seconds. It kept
the same adapter and SDK provider alive through 65 real minutes, confirmed that
the sensitive token value rotated only in memory, and queried again without
reconnecting. Initial job: `lazydb_53619a068b8742cf865be956f61ad282`; final job:
`lazydb_7c59f554e8ef465396188889c172f9c9`.

The remaining matrix cases below still require their exact environments:
service-account credentials, restricted IAM and confirmed cancellation of an
active remote job. Local HTTP fixtures and fixture-backed PTY checks do not
satisfy cloud checks.

### Credential and project matrix

Run the command above twice against the same disposable dataset, once with an
`authorized_user` ADC file and once with a `service_account` file. Set
`LAZYDB_TEST_BIGQUERY_CREDENTIALS_FILE` to the respective absolute path each time;
do not place either file in the repository. At least one run should use different
billing and data projects. Record only credential type, result, job IDs and
non-sensitive configuration; never copy the credential JSON or tokens.

The live test checks types, empty-result schema, native DDL, metadata, preview,
billing project/location and the billed-bytes cap. Successful runs must also
complete their exact UUID-table cleanup. It does not validate token expiry during
a live session, IAM denial, cancellation or the TUI; verify those separately.

### Real TUI workflow

The automated production PTY workflow uses the same five explicit variables as
the live integration test:

```sh
python3 -I scripts/ci/test-bigquery-live-tui.py --run-live --binary target/debug/lazydb
```

`--run-live` is mandatory. The workflow creates its own UUID-named table with a
one-hour expiration inside the designated existing dataset. Each query has a
10 MiB billed-byte cap and a 60-second timeout. Explorer searches for that exact
table before previewing it; unrelated tables are never previewed or dropped.
It exercises the real form/Test Connection, search, preview, DDL, console/history,
focus isolation and restart, then verifies native execution-project statistics.
Application files are temporary and the exact created table is dropped in cleanup.
The supplied ADC file is referenced by path and is never copied or deleted.
Cleanup failure is a failed run; expiration is a fallback after interruption.
The script does not create datasets/projects, enable APIs, change IAM or attach
billing, so prepare those outside the workflow with appropriate authorization.

For manual reproduction or checks beyond the automated path:

1. Start the production binary with an isolated application directory:
   `LAZYDB_CONFIG_HOME=/absolute/path/to/empty-test-home target/debug/lazydb`.
   Use a dedicated directory and preserve it until the restart check is done.
2. Create a BigQuery profile with the designated billing project, data project,
   dataset, explicit location and ADC file path. Set **Max billed bytes** to
   `10485760` and **Query timeout (secs)** to `60`.
3. Test Connection, save, quit normally and restart with the same `LAZYDB_CONFIG_HOME`.
   Confirm that projects, dataset, location and limits persist, with no password
   field or credential contents in the saved profile.
4. Expand the selected project/dataset and inspect a small table's nested columns,
   preview and native DDL. Use an existing synthetic fixture table owned by this
   test environment; the automated live test deletes its own table before exit.
5. Execute a constant query and a fully qualified query against that fixture.
   Confirm the grid/history and job statistics identify the billing project.
   Change Explorer focus and execute again in the existing console: its target
   must stay unchanged.
6. Cancel only a query already approved for this test dataset and cost limit.
   A fast query may finish before cancellation; this does not validate stopping
   an active job. Do not create a large scan just to make cancellation observable.
   When an active job is available, record the app's acceptance/final-state
   message and independently inspect that same job in BigQuery. Cancellation
   acceptance alone must never be recorded as confirmed termination.
7. Quit normally and confirm terminal echo/cursor and the alternate screen are
   restored. Remove only this workflow's isolated application directory when its
   saved state is no longer needed.

### Permission, location and billing boundaries

| Case | Required setup | Expected outcome |
| --- | --- | --- |
| Dataset access without project listing | An existing identity granted dataset access, plus job creation on the billing project; explicitly select project/dataset in Visible Objects. | Metadata and qualified reads work; discovery states that project listing is incomplete. |
| Missing job creation permission | An existing restricted identity without `bigquery.jobs.create` on the designated billing project. | Test Connection reports permission failure; it does not display an empty catalog as successful connection. |
| Dataset location mismatch | A copied test profile with a location different from the fixture dataset. | Fixture preview/query fails explicitly; no automatic location change or SQL replay. |
| Native billed-bytes cap | A small, approved synthetic table and a cap below its uncached scan requirement; ensure a cached result cannot bypass the intended check. | Google rejects the query for the native cap; reducing only local row/byte budgets is not treated as a billing limit. |
| Credential expiry | A disposable developer ADC session kept open beyond token expiry, if practical. | A later query refreshes through the SDK without manual access-token replacement. Record this separately from deterministic refresh tests. |

Use identities and resources already prepared for these cases. A missing
restricted identity or suitable fixture means that case remains unverified;
these instructions do not authorize changing IAM, enabling APIs, creating cloud
projects or increasing the approved scan budget.

## Explicit lifecycle acceptance

Four ignored library tests in `src/db/bigquery/live_tests.rs` cover the remaining
lifecycle cases without adding production APIs or endpoint overrides. Each also
requires its own `LAZYDB_TEST_BIGQUERY_RUN_*=1` flag. A missing flag or designated
credential file fails before connecting; ordinary CI never runs these tests.

For expiry and cancellation, set the existing explicit `PROJECT`, `LOCATION`
and `CREDENTIALS_FILE` variables. These two tests use no default dataset and need
no tables, so previously cleaned-up acceptance datasets need not be recreated.
Every job has a 10 MiB native billed-byte cap and a 30-second query timeout.

```sh
LAZYDB_TEST_BIGQUERY_RUN_EXPIRY=1 cargo test --lib --all-features \
  db::bigquery::transport::live_tests::bigquery_live_natural_token_expiry_same_connection \
  -- --ignored --exact --nocapture

LAZYDB_TEST_BIGQUERY_RUN_CANCEL=1 cargo test --lib --all-features \
  db::bigquery::transport::live_tests::bigquery_live_running_job_confirmed_stopped \
  -- --ignored --exact --nocapture
```

The expiry test requires `authorized_user` ADC. It keeps the same adapter and SDK
provider alive for **65 real minutes**, verifies token rotation by comparing
sensitive headers only in memory, and repeats a zero-scan constant query without
reconnecting. It does not simulate time or change token lifetimes. SDK refresh
may happen proactively; the final query must still succeed beyond the original
normal one-hour token lifetime. Only job IDs and elapsed-time messages are logged.

The cancellation test submits one fixed computation over 100 million generated
pairs, with no scanned tables, cache disabled and a 20-second native job timeout.
It records the original job ID before submission, observes `RUNNING`, invokes
the production cancellation path and independently polls that same job for
`DONE` with `errorResult.reason=stopped`. A query that completes before the
observation/cancellation, a different terminal error or an unknown final state
fails acceptance. Submission is never replayed. Cleanup addresses only that
original job. Native timeouts and cancellation are best effort, so an unknown
outcome still requires inspection of the recorded job ID.

To test a service account, designate an **existing, authorized** JSON file through
`LAZYDB_TEST_BIGQUERY_SERVICE_ACCOUNT_CREDENTIALS_FILE`, plus `PROJECT` and
`LOCATION`. This check executes only zero-scan constant queries:

```sh
LAZYDB_TEST_BIGQUERY_RUN_SERVICE_ACCOUNT=1 cargo test --lib --all-features \
  db::bigquery::transport::live_tests::bigquery_live_service_account_constant_query \
  -- --ignored --exact --nocapture
```

For restricted IAM, prepare an existing identity with job creation on `PROJECT`,
dataset-only access in a distinct `DATA_PROJECT`, and no `bigquery.jobs.create`
permission on a separately designated `LAZYDB_TEST_BIGQUERY_DENIED_PROJECT`.
Set `LAZYDB_TEST_BIGQUERY_RESTRICTED_CREDENTIALS_FILE`, `DATASET` and
`LAZYDB_TEST_BIGQUERY_RESTRICTED_TABLE` to an existing small, approved fixture.
The test checks that the data project is absent from native project listing,
that explicit dataset discovery, table metadata and a qualified limited read
work, and that Test Connection on the denied project reports the exact missing
job-creation permission. It never creates, modifies or drops the fixture.

```sh
LAZYDB_TEST_BIGQUERY_RUN_RESTRICTED=1 cargo test --lib --all-features \
  db::bigquery::transport::live_tests::bigquery_live_restricted_dataset_access_and_job_denial \
  -- --ignored --exact --nocapture
```

Do not grant permissions or create keys simply to satisfy these tests. If either
special identity is not already available, its case stays unverified. The
service-account constant check does not replace the separate table/catalog/type
matrix using that identity.

Record each case as passed, failed or unverified, with the binary revision/build,
credential type and relevant job ID. Leave cloud-backed TUI and authentication
claims unverified until those exact workflows have been exercised.
