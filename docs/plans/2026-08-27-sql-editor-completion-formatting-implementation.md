# SQL Editor Completion And Formatting Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restrict SQL completion to Insert mode, complete the selected/current SQL formatting workflow, and provide fully qualified relation labels with target- and dialect-aware insertion.

**Architecture:** Treat Insert-only completion as an App state invariant enforced at scheduling, calculation, acceptance, input mapping, and rendering boundaries. Pass a lightweight database/schema context into the SQL completion engine, where presentation and insertion are computed separately and dialect-specific namespace rules remain centralized. Reuse the existing scope resolver and safe formatter rather than creating a second formatting path.

**Tech Stack:** Rust 2024, Modalkit, Ratatui 0.30, Crossterm, sqlformat 0.5, sqlparser 0.62, Tokio, Cargo test, Clippy

**Reference:** `docs/plans/2026-08-27-sql-editor-completion-formatting-design.md`

**Worktree note:** `src/app.rs` and `tests/app_flow.rs` had unrelated uncommitted transaction-toggle changes when this plan was written. Preserve and integrate with those changes; stage only task-specific hunks in each commit.

---

### Task 1: Enforce The Insert-Only Completion Lifecycle

**Files:**
- Modify: `src/app.rs:1199-1267,3283-3434`
- Modify: `src/input/keymap.rs:140-175,208-238`
- Test: `tests/app_flow.rs`
- Test: `tests/keymap.rs:345-388`

**Step 1: Write failing lifecycle tests**

Add App tests that enter Normal mode, mutate SQL with `dd`, `x`, and `p`, and assert both that the expected text change occurred and `active_console().completion` remains `None`. Add a delayed-action test that records a `CompletionScheduleKey` in Insert mode, sends Escape, then dispatches `Action::CompletionDue(key)` and asserts no popup appears.

Add keymap tests proving `Ctrl-Space`, `Ctrl-n`, `Ctrl-p`, and Enter cannot invoke completion actions outside Insert mode even if stale popup state is injected.

**Step 2: Run the tests and verify the regression**

Run:

```bash
cargo test --test app_flow --test keymap --all-features
```

Expected: the Normal-mode mutation or delayed completion tests fail because `EditorEffect::Changed` and `CompletionDue` can call `complete_now`; stale popup key routing may also fail.

**Step 3: Add minimal mode guards**

In `App`, centralize the check with the existing `active_editor_mode()`:

```rust
fn completion_allowed(&self) -> bool {
    self.focus == Focus::Editor && self.active_editor_mode() == EditorMode::Insert
}
```

Use it to:

- clear popup and skip scheduling for a non-Insert `EditorEffect::Changed`;
- return `None` from `completion_key` outside Insert mode;
- clear popup and return no commands at the start of `complete_now` outside Insert mode;
- return no commands at the start of `accept_completion` outside Insert mode.

Update keymap completion routing and explicit `Ctrl-Space` mapping to require Insert mode. Do not change `EditorEffect::Changed`, revision tracking, or persistence behavior.

**Step 4: Run focused tests**

Run:

```bash
cargo test --test app_flow --test keymap --all-features
```

Expected: all tests pass, including the existing Escape and printable-input completion tests.

**Step 5: Commit the lifecycle fix**

```bash
git add src/app.rs src/input/keymap.rs tests/app_flow.rs tests/keymap.rs
git commit -m "fix(sql): restrict completion to insert mode"
```

### Task 2: Verify And Expose Selected Or Current SQL Formatting

**Files:**
- Modify: `src/help.rs:3-48,121-283`
- Modify: `src/app.rs:3283-3323,3445-3493`
- Test: `src/help.rs:360-415`
- Test: `tests/app_flow.rs`
- Test: `tests/sql_scope.rs`
- Reference: `src/sql/format.rs`

**Step 1: Write failing integration and Help tests**

Add a `HelpShortcutId::EditorFormat` expectation for an Editor row with key `Space f` and description `format selected / current SQL`.

Add App pipeline tests for:

- Normal mode with two statements: place the cursor in the second statement, press `Space f`, and assert only that statement is formatted.
- Visual Char and Visual Line selections: press `Space f` and assert only the contiguous selection changes.
- Visual Block: assert the buffer is unchanged and the FORMAT message explains that block formatting is unsupported.
- Cursor in a whitespace/comment gap: assert the buffer is unchanged and `No SQL scope at cursor` is shown.
- Successful formatting in Normal/Visual mode: assert no completion popup is created.
- `:format`: assert it reaches the same formatting path.

Use deterministic unformatted SQL such as `select a,b from users where a=1;` and derive expected output with `lazydb::sql::format_sql` where exact formatter whitespace would otherwise make the test brittle.

**Step 2: Run the focused tests and confirm failures**

Run:

```bash
cargo test --test app_flow --test sql_scope --all-features
cargo test help::tests --all-features
```

Expected: Help discovery fails; any uncovered App formatting behavior identifies the smallest required correction.

**Step 3: Implement the smallest formatting changes**

Add the Help shortcut row. Keep `App::format_current`, `EditorWorkspace::current_scope`, and `sql::format_sql` as the single path. Only adjust scope replacement or mode handling if a failing integration test demonstrates a defect. Preserve:

- `ScopeSource::Contiguous` replacement;
- Visual Block rejection;
- `ReplacementCursor::Start`;
- token-equivalence and procedural-body safety errors;
- original buffer contents on every error.

Do not add a new formatter, shortcut, or formatting configuration.

**Step 4: Run formatting-related tests**

Run:

```bash
cargo test --test app_flow --test sql_scope --all-features
cargo test help::tests --all-features
cargo test sql::format --all-features
```

Expected: all tests pass.

**Step 5: Commit formatting discoverability and coverage**

```bash
git add src/help.rs src/app.rs tests/app_flow.rs tests/sql_scope.rs
git commit -m "feat(sql): expose selected and current formatting"
```

### Task 3: Generate Qualified Relation Labels And Insertions

**Files:**
- Modify: `src/sql/completion.rs:18-45,141-258,275-280,362-453`
- Modify: `src/sql/mod.rs:18-24`
- Test: `tests/sql_completion.rs`

**Step 1: Add failing completion engine tests**

Build a fixture containing duplicate relation names in `app.public`, `app.audit`, and `analytics.bi`. Assert:

- every Table/View label is the sanitized full catalog path;
- PostgreSQL inserts `users`, `audit.users`, or `analytics.bi.users` relative to `app.public`;
- MySQL inserts `users` locally and `analytics.users` across databases without duplicate database/schema segments;
- SQLite inserts `users` locally and `archive.users` for another attached schema;
- Generic follows one-, two-, and three-part insertion rules;
- `app.public.us|` replaces only `us` with the object component;
- names such as `odd schema` and `odd"table` are quoted component by component;
- terminal control characters are sanitized in labels but preserved and quoted correctly in `insert_text`.

Update existing assertions that currently expect relation label `orders` or insertion `"users"`.

**Step 2: Run completion tests and verify failures**

Run:

```bash
cargo test --test sql_completion --all-features
```

Expected: relation labels are still bare and every relation insertion is still based only on `qualified_name.object`.

**Step 3: Introduce lightweight completion context**

Add and export:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CompletionContext<'a> {
    pub database: Option<&'a str>,
    pub schema: Option<&'a str>,
}
```

Replace `default_schema: Option<&str>` in `complete` with `context: CompletionContext<'_>`. Use `context.schema` for the existing schema ranking and compare namespaces case-insensitively.

**Step 4: Separate relation presentation from insertion**

Add small private helpers with these responsibilities:

```rust
fn relation_label(entry: &CatalogEntry) -> String;
fn relation_insert_text(
    entry: &CatalogEntry,
    context: CompletionContext<'_>,
    dialect: SqlDialect,
    qualifiers: &[String],
) -> String;
fn quoted_path<'a>(parts: impl IntoIterator<Item = &'a str>, dialect: SqlDialect) -> String;
```

Apply them only to Table/View candidates. Keep columns, databases, schemas, routines, and keywords on their existing object insertion behavior. When `qualifiers` is non-empty, return only the quoted object component because the replacement range excludes the qualifier already present in the buffer.

Implement PostgreSQL/Generic three-level behavior and MySQL/SQLite two-level namespace behavior exactly as approved in the design. Skip absent components rather than emitting empty path segments.

**Step 5: Run completion tests**

Run:

```bash
cargo test --test sql_completion --all-features
```

Expected: all contextual, alias, security, ranking, and qualified-insertion tests pass.

**Step 6: Commit the completion engine change**

```bash
git add src/sql/completion.rs src/sql/mod.rs tests/sql_completion.rs
git commit -m "feat(sql): qualify relation completion candidates"
```

### Task 4: Pass The Active Execution Target Into Completion

**Files:**
- Modify: `src/app.rs:3340-3415`
- Test: `tests/sql_completion.rs:333-375`
- Reference: `src/model/execution_target.rs`

**Step 1: Add failing App context tests**

Extend the existing active-console-target test to assert complete labels and insertion text for relations in the active schema, another schema, and another database. Dispatch `Action::CompletionAccept` for a PostgreSQL cross-database candidate and assert:

- the editor receives the three-part insertion;
- `active_console().execution_target` is unchanged.

**Step 2: Run the App completion tests**

Run:

```bash
cargo test --test sql_completion app_completion --all-features
```

Expected: compile failure until App passes `CompletionContext`, then behavior failure until the active target values are wired correctly.

**Step 3: Construct context from the active console target**

In `complete_now`, copy or borrow the active target namespace before mutably borrowing the tab:

```rust
let completion_context = self
    .active_console_opt()
    .and_then(|tab| tab.execution_target.as_ref())
    .map_or_default(|target| sql::CompletionContext {
        database: Some(target.database.as_str()),
        schema: target.schema.as_deref(),
    });
```

Adapt this sketch as needed for Rust lifetimes by retaining owned local `Option<String>` values before the `sql::complete` call. Remove the profile default-schema fallback because `ExecutionTarget::from_profile` already owns the editor's effective namespace; if no target exists, pass an empty context.

Do not mutate or switch `ExecutionTarget` from `accept_completion`.

**Step 4: Run App and completion tests**

Run:

```bash
cargo test --test sql_completion --all-features
cargo test --test app_flow --all-features
```

Expected: all tests pass.

**Step 5: Commit App integration**

```bash
git add src/app.rs tests/sql_completion.rs
git commit -m "feat(sql): use editor target for completion"
```

### Task 5: Add Defensive Popup Rendering And UI Coverage

**Files:**
- Modify: `src/ui/mod.rs:255-330`
- Test: `tests/ui_render.rs`

**Step 1: Write failing render tests**

Inject a non-empty popup into a SQL tab and render once in Insert mode and once after Escape. Assert the candidate appears only in Insert mode. Add a relation candidate with a long full label and assert the popup contains the complete path without clipping at the prior bare-name width.

**Step 2: Run UI tests and verify the failure**

Run:

```bash
cargo test --test ui_render --all-features
```

Expected: stale popup state is rendered in Normal mode.

**Step 3: Add the rendering guard**

Return before reading popup state unless `app.active_editor_mode() == EditorMode::Insert`. Keep width calculation based on `candidate.label` and `detail`; the newly complete label should therefore size the popup automatically. Preserve icon spans, muted detail styling, terminal sanitization, viewport anchoring, and selection contrast.

**Step 4: Run UI and keymap tests**

Run:

```bash
cargo test --test ui_render --test keymap --all-features
```

Expected: all tests pass.

**Step 5: Commit UI defense**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "fix(ui): hide completion outside insert mode"
```

### Task 6: Verify The Complete Change

**Files:**
- Reference: `docs/plans/2026-08-27-sql-editor-completion-formatting-design.md`
- Reference: `docs/plans/2026-08-27-sql-editor-completion-formatting-implementation.md`

**Step 1: Format the code**

Run:

```bash
cargo fmt --check
```

Expected: PASS. If it fails, run `cargo fmt`, inspect the formatting-only diff, and rerun the check.

**Step 2: Run focused suites**

Run:

```bash
cargo test --test app_flow --all-features
cargo test --test keymap --all-features
cargo test --test sql_completion --all-features
cargo test --test sql_scope --all-features
cargo test --test ui_render --all-features
```

Expected: all pass.

**Step 3: Run project validation**

Run:

```bash
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

Expected: all tests and Clippy pass with no whitespace errors.

**Step 4: Manually smoke-test the TUI**

Verify:

- type `select * from t` in Insert mode and inspect complete relation labels;
- accept local, cross-schema, and cross-database candidates;
- press Escape and use `dd`; no popup appears after the debounce interval;
- format a current statement with `Space f`;
- format Visual Char and Visual Line selections;
- search Help for `format` and find `Space f`.

Expected: behavior matches the approved design and the active execution target never changes on completion acceptance.

**Step 5: Commit any verification-only documentation adjustment**

Only if verification required a documentation correction:

```bash
git add docs/plans/2026-08-27-sql-editor-completion-formatting-*.md
git commit -m "docs(sql): clarify completion verification"
```
