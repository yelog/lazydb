# MongoDB Staged Delete Hardening Implementation Plan

**Goal:** Preserve PR #42's staged MongoDB relation deletion while preventing over-deletion, stale document targeting, duplicate submissions, and blocked deferred exits.

**Architecture:** Give each deletion a request UUID and keep the relation edit session busy until its matching result arrives. A confirmed deletion invalidates the DATA, BSON, and structure snapshots together, including background tabs, before any further document edit. Send one batch of single-document deletes guarded by complete loaded BSON snapshots, reusing the browser's existing compare/delete guard instead of assuming globally unique `_id` values.

**Tech Stack:** Rust, the existing reducer/command runtime, MongoDB BSON and the MongoDB Rust driver, cargo tests, isolated MongoDB 8.0 Docker fixtures.

## Task 1: Bound the database operation

Files: `src/db/mongodb/mutation.rs`, `tests/mongodb_mutation.rs`.

1. Add a live regression with two time-series measurements sharing one `_id`; deleting one must refuse without removing either measurement.
2. Validate the ordinary collection type, and build snapshot-guarded `limit: 1` statements before the write. Avoid cluster administration privileges and topology-specific assumptions.
3. Treat affected counts greater than the request as uncertain, and keep possibly partially applied server errors uncertain.
4. Cover ordinary collections, BSON identifiers, stale snapshots, collation, an empty request, and refusal behavior. Run the focused mutation tests against an isolated fixture.

## Task 2: Correlate and freeze pending deletions

Files: `src/action.rs`, `src/model/relation_edit.rs`, `src/app.rs`, `src/runtime.rs`, `tests/mongodb_relation.rs`.

1. Carry loaded BSON documents and a deletion request UUID in the command; store the pending UUID in the edit session and use its existing Busy mode. Return the UUID and original IDs with the result.
2. Refuse duplicate saves, rollback, and competing grid/document edits while the request is pending.
3. Ignore callbacks that do not match the pending request, tab, and tab generation; release Busy on a matching failure while retaining the marks.
4. Recheck the profile's current write policy in the runtime before invoking the adapter.
5. Add regressions for duplicate save, competing edit/delete/undo/discard, stale callbacks, and retry after a definite failure.

## Task 3: Keep snapshots and deferred actions consistent

Files: `src/app.rs`, `tests/mongodb_relation.rs`.

1. On success, clear the committed marks and invalidate DATA, BSON, and DDL together, advancing the tab generation to reject old loads.
2. Reload active DATA views; leave background DATA invalid so activation reloads it. Deleting while viewing DDL must also force a fresh DATA page before editing.
3. Complete a matching deferred intent after success, without starting a conflicting reload before a deferred quit. Release the deferred wait after failure.
4. Add regressions for background completion, completion in DDL, failed reload, and commit-on-quit success/failure.

## Task 4: Document and verify

Files: `docs/mongodb.md`, `docs/keybindings.md` if needed, this plan.

1. Document staged deletion and the supported collection boundaries.
2. Run `cargo test --locked -p lazydb --test mongodb_relation --test mongodb_mutation --test keymap` with a private local MongoDB fixture, plus focused model/runtime tests.
3. Run formatting, diff checks, and strict all-target/all-feature clippy; broaden regression tests when the shared edit-session changes warrant it.
4. Report the merged PR commit, the follow-up branch, validation results, and remaining limits. The user has authorized committing and pushing this follow-up, opening a PR, running actual TUI acceptance, and merging after CI passes.

## Execution status

- PR #42 was squash-merged to upstream `main` as `a59362f22ffcafac69ce724d6e51aaec5f3d8027`. Follow-up changes are on `codex/mongodb-staged-delete-hardening`.
- Replaced `_id`-only `deleteMany` with one ordered command of snapshot-guarded, single-document deletes. Added BSON-size and acknowledgement validation, ordinary-collection checks, conservative error handling, and current-profile authorization in the runtime.
- Added request correlation and pending-edit/close guards, invalidated all page representations after confirmed deletion, displayed the MongoDB command in review, and resumed or released deferred exits.
- Focused checks with private MongoDB 8.0 fixtures: 76 relation tests, 8 mutation tests, 169 keymap tests, and 2 documentation tests passed. The live tests include an authenticated user with only `readWrite` on one database and no cluster administration privileges.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`, formatting, and diff checks passed.
- `cargo test --locked --all-targets --all-features -- --quiet`: 3,864 passed, 0 failed, 30 ignored across 211 binaries. External database URL/requirement variables were removed only for that test process; environment-gated database cases followed the project's skip behavior. MongoDB live checks ran separately as described above.
- Time-series, views, capped collections, and unacknowledged writes remain explicitly unsupported for this deletion path. Publication proceeds through a separate repair PR with actual TUI acceptance and passing CI before merge.
- Follow-up PR: #50. Real PTY acceptance exposed that legacy terminals report uppercase `V` with the Shift modifier, which the relation grid previously ignored. Added a failing terminal-key regression, accepted Shift for browse/visual keys while preserving control/alt handling, and verified that real range selection deletes only the selected documents after confirmation.
