# Identifier Completion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add separator-insensitive SQL identifier completion and current-relation column suggestions to the relation preview `WHERE` and `ORDER BY` inputs.

**Architecture:** Introduce one small identifier matcher shared by SQL completion and the relation query bar. Keep SQL parsing and relation-query input state separate, source relation columns from the existing completion catalog with result metadata as a fallback, and share only popup presentation primitives.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm 0.29, SQLParser 0.62, existing LazyDB catalog/completion models and integration tests.

---

### Task 1: Add Separator-Insensitive Identifier Matching

**Files:**
- Create: `src/sql/identifier_match.rs`
- Modify: `src/sql/mod.rs`
- Test: `src/sql/identifier_match.rs`

**Step 1: Write failing matcher tests**

Add table-driven unit tests covering the exact supported boundary:

```rust
#[test]
fn classifies_exact_prefix_and_compact_prefix_matches() {
    assert_eq!(identifier_match("sys_user", "SYS_USER"), Some(IdentifierMatch::Exact));
    assert_eq!(identifier_match("sys_user", "sysu"), Some(IdentifierMatch::Prefix));
    assert_eq!(
        identifier_match("sys_user", "sysuser"),
        Some(IdentifierMatch::CompactPrefix)
    );
    assert_eq!(identifier_match("sys-user", "sysuser"), Some(IdentifierMatch::CompactPrefix));
    assert_eq!(identifier_match("sys user", "sysuser"), Some(IdentifierMatch::CompactPrefix));
}

#[test]
fn rejects_subsequences_suffixes_and_non_separator_compaction() {
    assert_eq!(identifier_match("sys_user", "syusr"), None);
    assert_eq!(identifier_match("sys_user", "user"), None);
    assert_eq!(identifier_match("sys$user", "sysuser"), None);
    assert_eq!(identifier_match("sys.user", "sysuser"), None);
}
```

**Step 2: Run the matcher tests and verify failure**

Run: `cargo test sql::identifier_match::tests --lib`

Expected: compilation fails because `identifier_match` and `IdentifierMatch` do not exist.

**Step 3: Implement the minimal matcher**

Create a module with an orderable match class:

```rust
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum IdentifierMatch {
    CompactPrefix,
    Prefix,
    Exact,
}

pub fn fold_identifier(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

pub fn compact_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            *character != '_'
                && *character != '-'
                && !character.is_ascii_whitespace()
        })
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn identifier_match(candidate: &str, query: &str) -> Option<IdentifierMatch> {
    let candidate = fold_identifier(candidate);
    let query = fold_identifier(query);
    if candidate == query {
        Some(IdentifierMatch::Exact)
    } else if candidate.starts_with(&query) {
        Some(IdentifierMatch::Prefix)
    } else if compact_identifier(&candidate).starts_with(&compact_identifier(&query)) {
        Some(IdentifierMatch::CompactPrefix)
    } else {
        None
    }
}
```

Treat an empty query consistently with existing prefix completion. Do not add edit distance, subsequence matching, normalization crates, or punctuation stripping.

Export only the symbols needed by `completion.rs` and the query-bar completion helper from `src/sql/mod.rs`; prefer `pub(crate)` unless integration tests require a public export.

**Step 4: Run the matcher tests and lint the module**

Run: `cargo test sql::identifier_match::tests --lib`

Expected: all matcher tests pass.

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: no warnings.

**Step 5: Commit the matcher**

```bash
git add src/sql/identifier_match.rs src/sql/mod.rs
git commit -m "feat(sql): add separator-insensitive identifier matching"
```

### Task 2: Integrate Compact Matching Into SQL Completion

**Files:**
- Modify: `src/sql/completion.rs:37-57,94-108,147-277,403-416,663-665`
- Modify: `src/sql/mod.rs`
- Test: `tests/sql_completion.rs`

**Step 1: Add failing SQL completion tests**

Extend the catalog fixture with relations whose names make ranking observable, including `sys_user` and `sysuser_archive`. Add tests equivalent to:

```rust
#[test]
fn relation_completion_ignores_identifier_separators() {
    let index = CompletionIndex::new(&fixture_with_system_tables());
    let sql = "select * from sysuser";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Table
            && candidate.label == "sys_user"
            && candidate.insert_text == "sys_user"
    }));
}

#[test]
fn ordinary_prefix_ranks_above_compact_prefix() {
    let candidates = /* complete `select * from sysuser` */;
    let labels = candidates
        .iter()
        .filter(|candidate| candidate.kind == CompletionKind::Table)
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(labels[..2], ["sysuser_archive", "sys_user"]);
}
```

Also add one alias-column case such as `select u.userid from users u` matching `user_id`, and verify candidates reachable through both indexes are returned once.

**Step 2: Run focused integration tests and verify failure**

Run: `cargo test --test sql_completion relation_completion_ignores_identifier_separators -- --exact`

Expected: fails because `sysuser` does not match `sys_user`.

Run: `cargo test --test sql_completion ordinary_prefix_ranks_above_compact_prefix -- --exact`

Expected: fails because compact candidates are absent.

**Step 3: Extend CompletionIndex and scoring**

Add `by_compact_name: BTreeMap<String, Vec<usize>>` and populate it in `rebuild()` with `compact_identifier(&entry.qualified_name.object)`.

Replace the boolean `CompletionScore.prefix` with a match-quality field:

```rust
pub struct CompletionScore {
    pub context: u8,
    pub name_match: u8,
    pub schema: u8,
}
```

Map `IdentifierMatch::{CompactPrefix, Prefix, Exact}` to increasing scores. Preserve context as the first field so relation/routine/column context continues to outrank unrelated candidate kinds.

Update keyword scoring to use the highest ordinary-prefix value. Keywords remain ordinary prefix only; do not compact SQL keywords.

**Step 4: Bound index scans and deduplicate results**

Replace the unbounded lower range with a helper that stops at the first non-prefix key:

```rust
fn prefixed_indices(
    names: &BTreeMap<String, Vec<usize>>,
    prefix: &str,
) -> impl Iterator<Item = usize> + '_ {
    names
        .range(prefix.to_owned()..)
        .take_while(move |(name, _)| name.starts_with(prefix))
        .flat_map(|(_, values)| values.iter().copied())
}
```

For unqualified, non-empty prefixes, merge ordinary and compact index results and deduplicate positions before inspecting entries. For parent-qualified completion, retain the `children[parent]` path and apply `identifier_match` directly. Avoid a second compact-index scan for an empty prefix.

Use `identifier_match(name, &prefix)` as the final acceptance and score source. Remove or replace the local `fold` helper so normalization has one definition.

**Step 5: Run SQL completion tests**

Run: `cargo test --test sql_completion`

Expected: all existing and new SQL completion tests pass, including contextual filtering, quoting, alias resolution, scope preference, and deduplication.

Run: `cargo test sql::completion --lib`

Expected: all completion unit tests pass.

**Step 6: Commit SQL integration**

```bash
git add src/sql/completion.rs src/sql/mod.rs tests/sql_completion.rs
git commit -m "feat(sql): support compact identifier completion"
```

### Task 3: Add Relation Query Column Completion State

**Files:**
- Modify: `src/model/data_query.rs`
- Modify: `src/model/text_input.rs`
- Modify: `src/sql/completion.rs`
- Modify: `src/app.rs:5400-5561`
- Test: `tests/relation_tabs.rs`

**Step 1: Add failing reducer tests for current-relation columns**

Build a relation app fixture with a descriptor and completion catalog entries for two relations. Add tests that assert:

- typing `userid` in relation `WHERE` creates a `user_id` candidate;
- columns belonging to the other relation are absent;
- the same behavior works in `ORDER BY`;
- ordinary prefix candidates rank above compact candidates;
- no completion is created for SQL-result query bars in this feature.

Use Catalog entries rather than manually assigning completion candidates so the test covers the real data path.

**Step 2: Run reducer tests and verify failure**

Run: `cargo test --test relation_tabs relation_query_suggests_current_relation_columns -- --exact`

Expected: compilation fails because `DataQueryState` has no completion state.

**Step 3: Add lightweight query completion models**

In `src/model/data_query.rs`, add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataQueryCandidate {
    pub name: String,
    pub type_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataQueryCompletion {
    pub candidates: Vec<DataQueryCandidate>,
    pub selected: usize,
    pub replace: crate::sql::TextRange,
}
```

Add `pub completion: Option<DataQueryCompletion>` to `DataQueryState`. Derive defaults through `Option::None`.

Add only the minimal `TextInput` replacement API needed to replace a character range and place the cursor after inserted text. Reuse an existing range-replacement method if one already exists; do not duplicate cursor boundary handling.

**Step 4: Expose relation columns through CompletionIndex**

Add a controlled accessor in `src/sql/completion.rs`:

```rust
pub fn relation_columns(
    &self,
    relation: &CatalogId,
) -> impl Iterator<Item = &CatalogEntry> {
    self.children
        .get(relation)
        .into_iter()
        .flatten()
        .filter_map(|position| self.entries.get(*position))
        .filter(|entry| entry.kind == CatalogKind::Column)
}
```

Keep index internals private.

**Step 5: Implement identifier extraction and candidate construction**

Add a private query-completion helper near the data-query reducer code in `src/app.rs`, or a focused function in `src/model/data_query.rs` if it remains independent of `App`.

The helper must:

- read the active input and cursor;
- find the current identifier range using Unicode-safe character boundaries;
- suppress suggestions inside single-quoted strings and active quoted identifiers;
- match only current relation columns using `identifier_match`;
- include native type from `CatalogMetadata::Column` when available;
- sort by match quality and deterministic name ordering;
- truncate to ten candidates;
- preserve or clamp selection when recomputing.

Initially use Catalog entries only. Add result-column fallback in Task 5 after the primary flow is covered.

Refresh completion after focus, insertion, deletion, clear, and cursor movement. Clear it on query submission, cancellation, tab/view transitions, and when no identifier or candidate exists.

**Step 6: Run reducer tests**

Run: `cargo test --test relation_tabs`

Expected: all relation tab tests pass, including existing query submission behavior.

**Step 7: Commit query completion state**

```bash
git add src/model/data_query.rs src/model/text_input.rs src/sql/completion.rs src/app.rs tests/relation_tabs.rs
git commit -m "feat(relation): suggest current table columns in query inputs"
```

### Task 4: Add Query Completion Keyboard Lifecycle

**Files:**
- Modify: `src/action.rs:204-228`
- Modify: `src/app.rs:647-797,5400-5561`
- Modify: `src/input/keymap.rs:1180-1241`
- Test: `tests/keymap.rs`
- Test: `tests/relation_tabs.rs`

**Step 1: Add failing keymap tests**

Create a relation query fixture with an open completion popup and assert:

```rust
assert_eq!(keymap.map(ctrl('n'), &app), Some(Action::DataQueryCompletionNext));
assert_eq!(keymap.map(ctrl('p'), &app), Some(Action::DataQueryCompletionPrevious));
assert_eq!(keymap.map(key(KeyCode::Tab), &app), Some(Action::DataQueryCompletionAccept));
assert_eq!(keymap.map(key(KeyCode::Esc), &app), Some(Action::DataQueryCompletionDismiss));
assert_eq!(keymap.map(key(KeyCode::Enter), &app), Some(Action::SubmitDataQuery));
```

Add companion assertions proving that, without a popup, `Tab` switches inputs and `Esc` cancels input as before.

**Step 2: Run keymap tests and verify failure**

Run: `cargo test --test keymap data_query_completion_keys_preempt_query_input_navigation -- --exact`

Expected: compilation fails because the completion actions do not exist.

**Step 3: Add actions and allow them on relation tabs**

Add only:

```rust
DataQueryCompletionNext,
DataQueryCompletionPrevious,
DataQueryCompletionAccept,
DataQueryCompletionDismiss,
```

Include these actions in the relation-tab allowlist at the beginning of `App::update`. Do not add relation-prefixed aliases.

**Step 4: Implement key precedence**

In `map_data_query`, when `query.completion.is_some()`, route `Ctrl+N`, `Ctrl+P`, `Tab`, and `Esc` to completion actions before normal query-input handling. Keep `Enter -> SubmitDataQuery` unconditionally.

All printable and editing keys continue through the current text input mapper so the reducer can recompute candidates.

**Step 5: Implement reducers and acceptance**

Implement wraparound or existing-popup-style selection for next/previous. Acceptance must:

- obtain the selected candidate safely;
- quote it with `sql::quote_identifier(name, self.sql_dialect())`;
- replace only `completion.replace` in the active input;
- place the cursor after inserted text;
- recompute or dismiss completion without reopening a stale candidate immediately.

Dismissal only clears `query.completion`. The next `Esc` then follows the existing cancel path.

**Step 6: Add reducer lifecycle tests**

In `tests/relation_tabs.rs`, verify:

- next/previous selection stays in bounds;
- `Tab` replaces `userid` with `user_id`;
- reserved or spaced names are dialect-quoted;
- surrounding text such as `userid desc, id asc` is preserved;
- first `Esc` dismisses candidates and second `Esc` restores submitted input;
- `Enter` submits while candidates are visible and clears completion.

**Step 7: Run focused tests**

Run: `cargo test --test keymap`

Expected: all keymap tests pass.

Run: `cargo test --test relation_tabs`

Expected: all reducer and lifecycle tests pass.

**Step 8: Commit keyboard lifecycle**

```bash
git add src/action.rs src/app.rs src/input/keymap.rs tests/keymap.rs tests/relation_tabs.rs
git commit -m "feat(relation): add query column completion controls"
```

### Task 5: Ensure Column Metadata Availability and Add Fallback

**Files:**
- Modify: `src/app.rs`
- Modify: `src/sql/completion.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/catalog_reducer.rs`

**Step 1: Add failing metadata scheduling test**

Use a connected app fixture with a selected relation whose child columns are absent. Trigger `Action::PreviewSelected` and assert that returned commands include both the relation preview request and the existing catalog request for:

```rust
CatalogTarget::RelationChildren {
    relation: descriptor.key.object_id.clone(),
}
```

If `PreviewSelected` already routes catalog expansion elsewhere, assert the actual public command shape rather than adding a parallel request type.

Add a second test proving no duplicate catalog request is scheduled when relation columns are already indexed or an equivalent request is pending.

**Step 2: Run scheduling test and verify failure**

Run: `cargo test --test relation_tabs opening_relation_preview_loads_missing_column_metadata -- --exact`

Expected: fails because only relation preview loading is currently scheduled.

**Step 3: Reuse the existing relation-children catalog pipeline**

At relation-tab creation/opening, inspect the completion index for current relation columns. When absent, create the existing catalog page request with `CatalogTarget::relation_children(...)` and the current connection/profile scope and generation.

Reuse existing request deduplication, stale response checks, pagination, and completion-index append behavior. Do not add a direct adapter call or store a second catalog cache on `RelationTab`.

If catalog loading cannot be scheduled because the app is offline, out of scope, or disconnected, still open the relation tab and preview normally.

**Step 4: Add result-column fallback tests**

Create a relation tab whose completion catalog has no columns but whose current or previous `RelationPreview` result set contains:

```rust
ColumnMeta {
    name: "user_id".into(),
    type_name: "bigint".into(),
}
```

Assert `userid` produces a candidate. Add a zero-row/empty-column test proving the query input remains editable and produces no popup rather than an error.

**Step 5: Implement fallback candidate collection**

Use Catalog relation columns first. If none exist, read columns from the active relation snapshot, including the preserved previous snapshot for loading, failed, or cancelled states. Reuse the existing snapshot/result access pattern rather than cloning an entire result set.

Deduplicate fallback columns by case-insensitive name. Catalog metadata remains authoritative when both sources exist.

**Step 6: Run catalog and relation tests**

Run: `cargo test --test relation_tabs --test catalog_reducer`

Expected: metadata scheduling, stale catalog response handling, fallback, and existing relation behavior all pass.

**Step 7: Commit metadata loading**

```bash
git add src/app.rs src/sql/completion.rs tests/relation_tabs.rs tests/catalog_reducer.rs
git commit -m "feat(relation): load column metadata for query completion"
```

### Task 6: Render Query Bar Completion and Verify End to End

**Files:**
- Modify: `src/ui/mod.rs:106-118,301-347,352-473`
- Modify: `src/ui/query_bar.rs`
- Modify: `src/ui/relation.rs`
- Modify: `src/ui/icons.rs` only if a generic column icon accessor is required
- Test: `tests/ui_render.rs`
- Test: `tests/sql_completion.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/keymap.rs`

**Step 1: Add failing UI tests**

Construct a relation tab with an active `WHERE` input and candidates. Assert:

- rendered output contains candidate name and type;
- `UiState.completion_popup` is set;
- the popup stays within the relation/results viewport at narrow sizes;
- terminal control characters in type details are rendered inertly;
- no popup is rendered when candidates are absent or query focus is inactive.

Retain existing SQL Editor popup anchor tests as regression coverage.

**Step 2: Run UI tests and verify failure**

Run: `cargo test --test ui_render relation_query_completion_is_anchored_to_active_input -- --exact`

Expected: fails because `query_bar::render` does not render completion candidates.

**Step 3: Return a cursor anchor from text input rendering**

Adjust `render_text_input` to return the sanitized on-screen cursor `Position` or add a narrowly scoped companion API. Update existing callers without changing visible text or cursor behavior.

Have `query_bar::render` expose the active query input anchor to `ui::relation`, together with a viewport boundary that excludes headers/footer and prevents popup overflow.

**Step 4: Extract a shared completion list renderer**

Refactor the existing SQL completion popup so generic placement and row rendering can be reused without forcing query candidates into `CompletionCandidate`.

A small interface is sufficient, for example prebuilt rows containing icon, label, detail, and selected state. Preserve:

- maximum ten rows;
- above/below placement logic;
- width clamping;
- selected accent style;
- `Clear` before `List`;
- existing SQL Editor `UiState.completion_popup` reporting.

Do not create a general widget framework.

**Step 5: Render relation query candidates**

Render column candidates after the relation body/query bar establishes its anchor, so the popup overlays the table rather than consuming query-bar layout space. Display the column icon, column name, and optional type.

Sanitize candidate label/detail using the same terminal-safe paths as other completion and catalog UI. Ensure query-bar candidates never render on SQL-result query bars in this scoped feature.

**Step 6: Run UI and feature tests**

Run: `cargo test --test ui_render`

Expected: all UI tests pass, including existing SQL Editor popup placement.

Run: `cargo test --test sql_completion --test relation_tabs --test keymap --test catalog_reducer`

Expected: all feature integration tests pass.

**Step 7: Run repository verification**

Run: `cargo fmt --all -- --check`

Expected: formatting check passes. If it fails, run `cargo fmt --all`, inspect the diff, and rerun the check.

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: no warnings.

Run: `cargo test --all-targets --all-features`

Expected: full test suite passes.

**Step 8: Inspect final changes**

Run: `git status --short`

Run: `git diff --check`

Run: `git diff -- src/sql src/model/data_query.rs src/model/text_input.rs src/action.rs src/app.rs src/input/keymap.rs src/ui tests/sql_completion.rs tests/relation_tabs.rs tests/keymap.rs tests/catalog_reducer.rs tests/ui_render.rs docs/plans/2026-08-28-identifier-completion-design.md docs/plans/2026-08-28-identifier-completion-implementation.md`

Expected: no whitespace errors; only intended identifier-completion changes are present. Do not revert or stage unrelated worktree changes.

**Step 9: Commit final rendering and verification changes**

```bash
git add src/ui/mod.rs src/ui/query_bar.rs src/ui/relation.rs src/ui/icons.rs tests/ui_render.rs
git commit -m "feat(ui): render relation query column suggestions"
```

Only include `src/ui/icons.rs` if it actually changed. Do not amend earlier commits if hooks fail; fix the issue and create a new commit.
