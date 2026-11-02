# BigQuery merge readiness

## Scope and submission boundaries

The PR candidate is based on GitHub main at
`ad33262bb14bdf8d3d1bea44b51ea13541f2105f`, which already includes native
OpenSearch support in PR #43. Its base tree is exactly identical to the previously
validated integration baseline at `5460c22`:
`a66cc1d2b973f6afa9e1a071a76514e8dbca63b0`.

A fresh `yelog/bigquery-pr-ready` branch/worktree contains the BigQuery delta
against that main commit. The original BigQuery worktree at `782a3b1` and the
integration worktree at `5460c22` are preserved unchanged. No commit, push, rebase,
reset or remote PR has been created. Checking GitHub main was read-only.

Freeze the native BigQuery driver, required shared dispatch/profile/SQL changes,
correctness regressions and documentation as one coherent feature candidate:
64 modified tracked files and 19 new files, 83 files total. The new driver,
GoogleSQL module, deterministic tests, opt-in live tests and all three PTY scripts
must be included; a tracked-only diff is incomplete. Preserve the native
cancellation-owner and cross-driver credential regressions needed alongside the
already merged OpenSearch implementation.

Keep the four explicitly gated cloud lifecycle tests with their `cfg(test)`
module declaration and fixture instructions. They add no production endpoint
override, dependency or public API. Reviewers can examine them separately without
omitting the files required by that module declaration.

The local review package records the exact base, complete file inventory,
SHA-256 content hashes, a patch including new files, validation evidence and a
ready-to-paste PR description. Its freeze identifies source contents; it does not
change the package version or create a release tag.

Optional tokenizer/completion efficiency cleanups are outside this frozen scope.

## Readiness review and fixes

The independent correctness review fixed seven issues, with regressions:

- Bodyless native cancellation POSTs now explicitly send `Content-Length: 0`.
  Actual Google requests had returned HTTP 411; GET requests remain unchanged.
- BigQuery read-only policy is checked before confirmation, after confirmation,
  against the runtime's latest profile and in the adapter. Policy changes retire
  the old connection.
- Unknown BigQuery job outcomes retain `Unknown` certainty in history. A known
  submission rejection retains its original error and makes no cancellation
  request or speculative billing warning.
- Switching through BigQuery marks old stored credentials for deletion even if
  the draft switches back to a password-based driver before saving.
- URL import/export preserves read-only state and rejects conflicting aliases.
- Preview sort columns use GoogleSQL backticks, including native escapes.
- Preview filters reuse the shared GoogleSQL compatibility parser.

The same standalone fixes are backported to the original BigQuery worktree.
The original runtime adaptation preserves existing Elasticsearch behavior.
Review-focused checks passed 120 tests with five ignored cloud/PTY cases before
the final combined suite and standalone validation were launched.

## PR draft

Title: `feat(bigquery): add native GoogleSQL and refreshable ADC workflows`

LazyDB can connect to Google BigQuery with developer ADC or service-account JSON
through the official Google Cloud authentication library. Query jobs use the
selected execution/billing project, while catalog discovery, default datasets
and qualified reads can use a different data project. The connection form, URL
import/export, persistence, Test Connection, Explorer, native preview/DDL,
GoogleSQL console and agent workflows share these settings.

The native REST adapter retains job identity across uncertain submissions,
checks the original job instead of replaying SQL, polls and pages results, and
reports native statistics and cancellation state. Every query has a native
billed-bytes cap and job timeout, with separate local response/result budgets.
Credential contents and access tokens are excluded from persisted profiles and
URLs. Conservative statement classification and read-only/write-confirmation
policies apply to execution.

The PR is based on main after native OpenSearch support was merged. The final
main-based candidate passed **3,803 tests, zero failures and 30 ignored cases**
across 210 reported binaries with `cargo test --locked --all-targets --all-features`.
Formatting, all-target/all-feature Clippy with warnings denied and the locked
all-feature production build also passed. Fresh PTY runs passed the fixture-backed
connect/catalog/preview/DDL/query workflow and production profile/save/restart,
including temporary-file cleanup and terminal restoration. All three new Python
scripts passed syntax checks; locked offline Cargo metadata discovers the new
test targets and retains `google-cloud-auth =1.4.0` with default features disabled.

Real developer-ADC verification passed native types, pagination, billing and
location boundaries, production TUI/CLI workflows, and both same-project and
cross-project execution. A 65-real-minute test retained the same adapter and SDK
provider, confirmed token rotation only in memory and queried successfully again
without reconnecting. Production read-only URL checks also rejected DML locally.

Service-account authentication is implemented but remains unverified against
Google Cloud. Restricted-IAM behavior needs an existing designated test identity.
Native cancellation request acceptance was verified, but the tested jobs finished
normally; confirmed remote termination remains unverified. The adapter preserves
that distinction and never replays SQL when the original job outcome is unknown.

🤖 Generated with [Claude Code](https://claude.com/claude-code)

## Current lifecycle evidence

On 2026-11-02, the natural-expiry run passed: one test, zero failures, total
runtime 3,903.17 seconds. It retained the same adapter and SDK provider for 65 real
minutes, observed token rotation using an in-memory sensitive-header comparison,
and executed the final constant query without reconnecting or simulating time.
Both queries verified zero processed bytes, the configured execution project,
location and native billing cap. Initial job:
`lazydb_53619a068b8742cf865be956f61ad282`; final job:
`lazydb_7c59f554e8ef465396188889c172f9c9`. Credential contents and token values
were not printed. Evidence is in `cloud-natural-expiry.log` beside the other
non-sensitive validation logs.

The first active-cancellation runs observed RUNNING but Google rejected the empty
POST with HTTP 411. The explicit zero content-length fix removed that rejection.
Two post-fix runs observed RUNNING and confirmed cancellation request acceptance,
but the original jobs eventually reached DONE without error, rather than reason
`stopped`. Independent native inspection confirmed both jobs were terminal with
zero processed bytes. The 100-million-pair fixture also completed normally;
computation size was not escalated further. These are failed termination
acceptance checks, not proof of a remotely stopped job. No SQL was replayed, and
no tables, datasets, billing or IAM resources were changed by these lifecycle
runs. Confirmed active-job termination remains unverified.

Neither `LAZYDB_TEST_BIGQUERY_SERVICE_ACCOUNT_CREDENTIALS_FILE` nor
`LAZYDB_TEST_BIGQUERY_RESTRICTED_CREDENTIALS_FILE` was configured. Those cases
remain unverified. The test entrypoints document their exact prerequisites and
do not create keys, modify IAM, enable billing or create cloud resources.

## Deferred acceptance and efficiency work

- Service-account cloud acceptance: use an existing authorized JSON file for the
  zero-scan opt-in test, then the documented table/catalog/type matrix. Record
  credential type and job IDs without copying credentials. Missing designated
  credentials leave this case unverified.
- Restricted IAM: use the existing opt-in test with an identity that has
  dataset-only access, permitted job creation in one project and denied job
  creation in another. Pass only after explicit-scope metadata/read succeeds and
  the denied project reports the missing `bigquery.jobs.create` permission.
- Confirmed active-job termination: reuse a suitably bounded, approved workload,
  observe RUNNING and inspect the original job until DONE with reason `stopped`.
  A normally completed job or accepted cancellation request does not pass. Do not
  increase scan/computation limits merely to force a cancellation observation.
- GoogleSQL tokenizer efficiency: avoid repeated full-input tokenization while
  retaining raw/triple strings, escaped identifiers and ARRAY/STRUCT behavior.
- Completion efficiency: compute the unchanged identifier replacement span once
  per request rather than once per table candidate, preserving quoted/project
  identifier completion boundaries.

These are local follow-up records, not newly created public issues. They do not
expand the frozen implementation, grant cloud permissions or authorize releases.
