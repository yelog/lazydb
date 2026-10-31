# MongoDB Explorer and Field Editors Implementation Plan

**Goal:** Resolve upstream issues #27 and #25 using the existing contributor patches while preserving compare-and-replace document saving.

**Architecture:** Keep collection activation separate from tree expansion. Give document fields single-line or JSON buffers, retain the whole-document EJSON draft as the source of truth, and bound every editor to its actual viewport. Scalar field navigation validates and applies locally before opening the next editable field.

**Tech Stack:** Rust, ratatui, MongoDB BSON, chrono, existing TextInput and JsonBuffer.

## Task 1: Collection activation (#27)

- Apply upstream PR #28 to `src/app.rs`, `tests/mongodb_explorer.rs`, `tests/mongodb_relation.rs`, `docs/keybindings.md`, and `docs/mongodb.md` without committing.
- Add coverage for activation of an expanded collection, repeated activation reusing the tab, and mouse-equivalent primary/toggle actions.
- Run `cargo test --locked --test mongodb_explorer --test mongodb_relation`.

## Task 2: Field editor patches (#25)

- Apply PR #26, then the date and documentation patches from commits `ec2f6d9` and `3c903a8`.
- Preserve upstream document conflict handling and the original BSON `expected` snapshot.
- Run the relevant relation and keymap tests to identify integration issues.

## Task 3: Correct editor boundaries

- In `src/ui/relation.rs`, use the actual layout value rectangle, clamp dimensions before converting to u16, measure terminal cell widths, and handle small terminals without losing the editable surface.
- In `src/model/relation_edit.rs` and `src/app.rs`, validate/apply scalar drafts before switching fields, skip `_id`, stop at either end, and retain invalid drafts.
- Add regression coverage in `tests/mongodb_relation.rs` for long content scrolling, cursor bounds, resize, Unicode, navigation, invalid input, date precision/range, and BSON preservation.
- Keep formatting, cancel, and field application local; document saving remains one guarded replacement.

## Task 4: Help, docs, and verification

- Update `src/help.rs`, `docs/keybindings.md`, and editor feature notes to describe field navigation and the distinction between applying a field and saving a document.
- Run focused tests including `keymap`, `mongodb_relation`, `mongodb_explorer`, `mongodb_values`, `docs`, `ui_render`, and help tests.
- Run `cargo fmt --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, and `cargo test --locked --all-targets --all-features` with an available DuckDB runtime.
- Report environment-gated database checks accurately. Leave changes on the feature branch without commit, push, or remote PR creation.

## Execution status

- Collection activation and field/date/docs patches applied without commits on `fix/mongodb-explorer-field-editors`.
- Corrected viewport bounds, Unicode widths, coordinate conversion, small-terminal layout, scalar draft navigation, and date precision/range handling.
- Focused integration binaries: 598 passed, 1 ignored. Field viewport unit tests: 2 passed. Help unit tests: 47 passed.
- `cargo fmt --check`, `git diff --check`, and strict all-target/all-feature clippy passed.
- First full-suite attempt reached 2,335 passed before 8 PostgreSQL adapter tests failed because the configured database connection timed out. The second full all-target/all-feature run removed only process-local external database test settings and supplied a verified DuckDB runtime: 3,461 passed, 0 failed, 16 ignored across 189 binaries. That run did not exercise environment-gated external database tests.
- Live database follow-up (2026-10-31): confirmed the inherited PostgreSQL setting pointed to a remote instance rather than the local Docker fixture. Overrode it only for the test process with the local `lazydb-pg-ci-fix` instance on port 5433; credentials were read privately from container configuration and not logged. PostgreSQL adapter and principal suites: 25 passed, including all 8 previously timed-out tests.
- Started the current project's existing `lazydb-mongodb-rs-1`, `lazydb-mongodb-standalone-1`, and `lazydb-mongodb-principals-1` containers on ports 27017/27018/27019. Restored their missing `lazydb_default` network connections and service aliases, initialized `lazydb-rs`, and verified the writable primary and healthy containers. Existing volumes were retained.
- Ran the 14 MongoDB CI test binaries plus both PostgreSQL binaries with `--locked --all-features -- --test-threads=1 --nocapture` and `LAZYDB_REQUIRE_DATABASE_TESTS=1`: 108 passed, 0 failed, 0 ignored, no skip messages (83 MongoDB + 25 PostgreSQL). The live `relation_document_compare_and_replace_preserves_concurrent_updates` regression passed. Containers remained running after validation; no commit or push was performed during that validation step.

- Pre-commit review found that relaxed EJSON date strings could bypass the millisecond precision check in the BSON parser. Added failing regressions for submillisecond relaxed EJSON and leap seconds, then routed relaxed `$date` strings through the same checked timestamp conversion as plain RFC3339 input. Canonical integer-millisecond EJSON remains supported.
- Review also found that preserving an outer margin could hide the field editor when the available document area was only three or four cells high/wide. The field popup now uses that full dimension when necessary to retain an editable cell; resize regressions include a four-row area and a 3×3 area.
- Post-review verification: 622 related tests passed with 1 existing ignored test after the date fix, with live MongoDB fixtures and no skip messages. After the small-pane fix, 2 field viewport unit tests, 47 help tests, 2 documentation tests, and all 49 MongoDB relation tests passed; formatting, diff checks, and strict all-target/all-feature clippy also passed.

## Sources

- Collection activation: https://github.com/yelog/lazydb/pull/28 (commit `596a8c3`).
- Field modal: https://github.com/yelog/lazydb/pull/26 (commit `9c7c1aa`).
- Date editor: contributor commit `ec2f6d9`.
- Keybinding documentation: contributor commit `3c903a8`.
- Consolidated contributor work: https://github.com/gonzalobuassodev/lazydb/pull/11.
