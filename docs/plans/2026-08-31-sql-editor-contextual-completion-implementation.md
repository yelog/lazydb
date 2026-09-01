# SQL Editor Contextual Completion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make SQL Editor expression completion show only visible relation columns, expression-capable functions, and position-valid keywords while preserving relation and qualified catalog completion.

**Architecture:** Keep completion as a pure service in `src/sql/completion.rs`. First replace the broad general context with strict cursor-position allowlists and context-specific keyword sets. Then replace whitespace-based relation discovery with a tolerant lexical analyzer that tracks clauses, aliases, and nested query scopes without requiring valid complete SQL.

**Tech Stack:** Rust 2024, existing `CatalogEntry`/`CompletionIndex` model, existing SQL statement scanner, existing identifier matching, Cargo integration tests.

---

## Constraints

- Preserve the public `sql::complete` signature and `CompletionCandidate` contract.
- Preserve relation insertion, quoting, terminal sanitization, popup lifecycle, and the ten-row limit.
- Do not add a new parser dependency. `sqlparser` remains available elsewhere, but completion must tolerate incomplete SQL.
- Do not modify unrelated current worktree changes in `src/ui/query_bar.rs`, `tests/ui_render.rs`, or other plan documents.
- Do not commit unless the user explicitly requests a commit. The checkpoints below identify logical commit boundaries only.

## Task 1: Lock Down Expression Candidate Policy

**Files:**
- Modify: `tests/sql_completion.rs`
- Modify: `src/sql/completion.rs:157-295`

**Step 1: Add a reusable test fixture with several visible and unrelated columns**

Extend `tests/sql_completion.rs` with a fixture containing:

- table `sys_user` with `update_time`, `update_user`, `update_user_phone`,
  `user_type`, and `username` columns;
- unrelated table `user_agreement_accept` with an unrelated column;
- at least one unrelated table beginning with `u` so prefix filtering can prove
  that relation candidates are excluded rather than merely ranked lower.

Use the existing `CatalogEntry::relation` and `CatalogEntry::relation_child`
construction style from `fixture()` and `compact_match_fixture()`.

**Step 2: Write failing tests for the two reported cases**

Add tests equivalent to:

```rust
#[test]
fn select_expression_excludes_relation_candidates() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "select u from sys_user";
    let candidates = complete(
        sql,
        "select u".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Column && candidate.label == "username"
    }));
    assert!(candidates.iter().all(|candidate| !matches!(
        candidate.kind,
        CompletionKind::Database
            | CompletionKind::Schema
            | CompletionKind::Table
            | CompletionKind::View
    )));
    assert!(!candidates.iter().any(|candidate| candidate.label == "UPDATE"));
}

#[test]
fn where_expression_excludes_relation_candidates() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "select * from sys_user\nwhere ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Column && candidate.label == "update_time"
    }));
    assert!(candidates.iter().all(|candidate| !matches!(
        candidate.kind,
        CompletionKind::Database
            | CompletionKind::Schema
            | CompletionKind::Table
            | CompletionKind::View
    )));
}
```

**Step 3: Run the focused tests and verify they fail**

Run:

```bash
cargo test --test sql_completion select_expression_excludes_relation_candidates
cargo test --test sql_completion where_expression_excludes_relation_candidates
```

Expected: both tests fail because table candidates are still admitted by
`Context::General`; the first test also observes the statement-level `UPDATE`
keyword.

**Step 4: Replace `Context::General` with explicit positions**

In `src/sql/completion.rs`, introduce private position types:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Context {
    Statement,
    Relation,
    Expression(ExpressionContext),
    Qualifier,
    Routine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpressionContext {
    Projection,
    Predicate,
    Grouping,
    Ordering,
    Returning,
}
```

Add a private allowlist helper:

```rust
fn catalog_kind_allowed(context: Context, kind: CompletionKind) -> bool {
    match context {
        Context::Statement => false,
        Context::Relation => matches!(
            kind,
            CompletionKind::Database
                | CompletionKind::Schema
                | CompletionKind::Table
                | CompletionKind::View
        ),
        Context::Expression(_) => {
            matches!(kind, CompletionKind::Column | CompletionKind::Function)
        }
        Context::Qualifier => matches!(
            kind,
            CompletionKind::Column | CompletionKind::Table | CompletionKind::View
        ),
        Context::Routine => {
            matches!(kind, CompletionKind::Function | CompletionKind::Procedure)
        }
    }
}
```

Apply this helper before candidate construction. Keep the existing unqualified
column-to-binding check. Remove the three duplicated context kind checks once
the allowlist covers them.

Initially classify:

- `FROM`, `JOIN`, `UPDATE`, and `INTO` continuations as `Relation`;
- `CALL` and `EXECUTE` continuations as `Routine`;
- `SELECT` projection text as `Expression(Projection)`;
- `WHERE`, `ON`, and `HAVING` as `Expression(Predicate)`;
- `GROUP BY` as `Expression(Grouping)`;
- `ORDER BY` as `Expression(Ordering)`;
- `RETURNING` as `Expression(Returning)`;
- text before a recognized statement keyword as `Statement`.

This first classification may use the existing lowercase-before-cursor scan.
The lexical scanner in later tasks will replace it.

**Step 5: Update context scoring without weakening the allowlist**

Use a small private helper rather than an open-ended fallback:

```rust
fn catalog_context_score(context: Context, kind: CompletionKind) -> u8 {
    match (context, kind) {
        (Context::Relation, CompletionKind::Table | CompletionKind::View)
        | (Context::Qualifier, CompletionKind::Column)
        | (Context::Expression(_), CompletionKind::Column)
        | (Context::Routine, CompletionKind::Function | CompletionKind::Procedure) => 3,
        _ => 2,
    }
}
```

Candidate kinds rejected by `catalog_kind_allowed` must never reach this helper.

**Step 6: Run the focused tests**

Run:

```bash
cargo test --test sql_completion select_expression_excludes_relation_candidates
cargo test --test sql_completion where_expression_excludes_relation_candidates
```

Expected: relation-kind assertions pass. The `UPDATE` assertion may remain
failing until Task 2, which is acceptable only if the relation filtering part is
confirmed separately.

**Logical checkpoint:** expression positions use a strict catalog-kind allowlist.

## Task 2: Add Position-Specific Keyword Completion

**Files:**
- Modify: `tests/sql_completion.rs`
- Modify: `src/sql/completion.rs:265-295, 677-694`

**Step 1: Add failing keyword-policy tests**

Cover at least these cases:

```rust
#[test]
fn statement_and_expression_keywords_are_contextual() {
    let index = CompletionIndex::default();

    let statement = complete(
        "u",
        1,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(statement.iter().any(|candidate| candidate.label == "UPDATE"));

    let projection_sql = "select u from users";
    let projection = complete(
        projection_sql,
        "select u".len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(!projection.iter().any(|candidate| candidate.label == "UPDATE"));
}

#[test]
fn predicate_completion_offers_predicate_keywords() {
    let index = CompletionIndex::default();
    let sql = "select * from users where n";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(candidates.iter().any(|candidate| candidate.label == "NOT"));
}
```

Also update `general_sql_keywords_rank_before_matching_catalog_names`; its
current assumption that every prefix is a general statement context should be
renamed and restricted to statement-start behavior.

**Step 2: Run tests and verify they fail**

Run:

```bash
cargo test --test sql_completion statement_and_expression_keywords_are_contextual
cargo test --test sql_completion predicate_completion_offers_predicate_keywords
```

Expected: projection incorrectly includes `UPDATE`, and predicate completion
lacks `NOT`.

**Step 3: Replace the global keyword list with context-specific sets**

Implement:

```rust
fn keywords(context: Context, dialect: SqlDialect) -> &'static [&'static str] {
    match context {
        Context::Statement => match dialect {
            SqlDialect::MySql => &["SELECT", "INSERT", "UPDATE", "DELETE"],
            _ => &["SELECT", "WITH", "INSERT", "UPDATE", "DELETE"],
        },
        Context::Expression(ExpressionContext::Projection) => {
            &["DISTINCT", "CASE", "NULL", "TRUE", "FALSE"]
        }
        Context::Expression(ExpressionContext::Predicate) => &[
            "AND", "OR", "NOT", "EXISTS", "IN", "IS", "NULL", "LIKE", "BETWEEN",
            "CASE", "TRUE", "FALSE",
        ],
        Context::Expression(ExpressionContext::Grouping) => {
            &["HAVING", "CASE", "NULL"]
        }
        Context::Expression(ExpressionContext::Ordering) => match dialect {
            SqlDialect::MySql => &["ASC", "DESC"],
            _ => &["ASC", "DESC", "NULLS FIRST", "NULLS LAST"],
        },
        Context::Expression(ExpressionContext::Returning) => {
            &["CASE", "NULL", "TRUE", "FALSE"]
        }
        Context::Relation => &["LATERAL"],
        Context::Qualifier | Context::Routine => &[],
    }
}
```

The exact initial word lists may be reduced if a keyword is not valid for all
supported dialects, but statement-level DML keywords must not leak into
expression contexts.

Call `keywords(context, dialect)` from `complete`. Keep qualifier suppression
and prefix matching. Assign keyword context scores explicitly:

- statement keywords: `4`;
- expression keywords: `2` so visible columns lead an empty-prefix popup;
- relation modifiers: `1`;
- no qualifier or routine keywords.

**Step 4: Run all SQL completion tests**

Run:

```bash
cargo test --test sql_completion
```

Expected: all tests pass, including the two reported-case tests from Task 1.

**Logical checkpoint:** simple SQL now has semantically filtered catalog and
keyword candidates.

## Task 3: Introduce a Tolerant Completion Lexer

**Files:**
- Modify: `src/sql/completion.rs:464-665`
- Modify: `tests/sql_completion.rs`

**Step 1: Add failing tests for false keywords and bindings**

Add behavior tests proving that strings and comments do not alter context or
relations:

```rust
#[test]
fn strings_and_comments_do_not_create_relation_bindings() {
    let index = CompletionIndex::new(&contextual_fixture());
    for sql in [
        "select 'from user_agreement_accept' as note from sys_user where u",
        "select 1 /* join user_agreement_accept */ from sys_user where u",
        "select 1 -- join user_agreement_accept\nfrom sys_user where u",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );
        assert!(candidates.iter().all(|candidate| {
            candidate.kind != CompletionKind::Column
                || candidate.label != "unrelated_column"
        }));
    }
}
```

Add quoted-identifier coverage appropriate to PostgreSQL and MySQL.

**Step 2: Run the tests and verify failure**

Run:

```bash
cargo test --test sql_completion strings_and_comments_do_not_create_relation_bindings
```

Expected: at least one case fails because `relation_bindings` splits raw text
without understanding strings or comments.

**Step 3: Add private lexical token types**

Keep these private to `completion.rs`:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
enum CompletionTokenKind {
    Word(String),
    Dot,
    Comma,
    LeftParen,
    RightParen,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompletionToken {
    kind: CompletionTokenKind,
    start: usize,
    end: usize,
    depth: usize,
}
```

Implement:

```rust
fn completion_tokens(text: &str, dialect: SqlDialect) -> Vec<CompletionToken>
```

The scanner must process UTF-8 safely while retaining byte offsets. It must:

- consume single-quoted strings, including doubled quote escapes;
- consume PostgreSQL/SQLite double-quoted identifiers;
- consume MySQL backtick identifiers;
- consume `--` line comments and `/* ... */` block comments;
- emit quoted identifiers as `Word` tokens without surrounding quotes;
- emit unquoted identifier words using the existing identifier character policy;
- emit dot, comma, and parentheses;
- assign each token its nesting depth;
- tolerate unterminated strings, identifiers, and comments by consuming to the
  available end rather than returning an error.

Do not add operator tokens until a context rule needs them.

**Step 4: Add direct unit tests for scanner edge cases if needed**

Because the scanner is private, place `#[cfg(test)]` unit tests at the end of
`src/sql/completion.rs` only for lexical cases that are cumbersome to establish
through public completion behavior. Prefer integration tests for user-visible
behavior.

**Step 5: Make existing context and binding code consume tokens**

Change `context_at` and `relation_bindings` to consume a shared token slice
rather than independently scanning raw text. In `complete`, tokenize the current
statement once and reuse the result.

Do not implement nested visibility yet. At this checkpoint, preserve the
existing flat binding behavior while eliminating false tokens from strings and
comments.

**Step 6: Run focused and full tests**

Run:

```bash
cargo test --test sql_completion strings_and_comments_do_not_create_relation_bindings
cargo test --test sql_completion
```

Expected: all pass.

**Logical checkpoint:** context and relation discovery are lexical rather than
raw whitespace scans.

## Task 4: Resolve Joins, Comma Relations, And Aliases

**Files:**
- Modify: `src/sql/completion.rs:525-661`
- Modify: `tests/sql_completion.rs`

**Step 1: Add failing multi-relation tests**

Create a fixture with `users u` and `roles r`, each with a uniquely named column
and a shared `id` column. Add tests for:

```rust
#[test]
fn join_predicate_sees_both_relation_bindings() {
    // `select * from users u join roles r on |`
    // Assert unique columns from both tables are present.
    // Assert unrelated-table columns and relation candidates are absent.
}

#[test]
fn comma_from_list_sees_each_relation_binding() {
    // `select | from users u, roles r`
    // Assert unique columns from both tables are present.
}

#[test]
fn alias_qualified_completion_only_uses_that_binding() {
    // `select r.| from users u join roles r on ...`
    // Assert role columns are present and user-only columns are absent.
}
```

**Step 2: Run tests and verify the comma-list case fails**

Run:

```bash
cargo test --test sql_completion join_predicate_sees_both_relation_bindings
cargo test --test sql_completion comma_from_list_sees_each_relation_binding
cargo test --test sql_completion alias_qualified_completion_only_uses_that_binding
```

Expected: comma-separated relation discovery fails; alias behavior may expose
additional parser defects.

**Step 3: Replace tuple bindings with a private relation binding type**

Implement:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
struct RelationBinding {
    name: Vec<String>,
    alias: Option<String>,
    depth: usize,
}
```

Parse qualified relation names as components instead of discarding all but the
last component. Accept aliases with optional `AS`. Reject reserved clause and
join keywords as aliases.

After `FROM`, parse another relation after a comma at the same depth. After
`JOIN`, parse its relation independently. Stop the current relation list at
`WHERE`, `ON`, `GROUP`, `HAVING`, `ORDER`, `LIMIT`, `RETURNING`, set operators,
or a lower nesting depth.

**Step 4: Resolve bindings to catalog IDs deterministically**

Replace `relation_parents(index, relation: &str)` with:

```rust
fn relation_ids(
    index: &CompletionIndex,
    binding: &RelationBinding,
    context: CompletionContext<'_>,
) -> Vec<CatalogId>
```

Resolution order:

1. match every supplied qualification component against the catalog entry's
   database, schema, and object path;
2. for unqualified names, prefer entries matching the active database and
   schema;
3. if that produces no match, return all object-name matches rather than
   selecting the first arbitrary match.

Update `entry_belongs_to_binding`, `qualified_candidate_indices`, and
`relation_ids_for_completion` to use resolved `CatalogId` values. Remove alias
comparison against the catalog relation's object name; aliases identify
bindings, not catalog entries.

Pass `CompletionContext` into any private resolution helper. If
`relation_ids_for_completion` needs the active target to apply the same
resolution, change its public signature to accept `CompletionContext` and update
the single call in `src/app.rs:5384`. Prefer this small explicit API change over
duplicated, inconsistent resolution.

**Step 5: Deduplicate columns by catalog identity**

When multiple bindings resolve to the same relation, collect entry positions or
catalog IDs through a `HashSet` before constructing candidates. Do not dedupe
same-named columns from genuinely different visible relations unless the popup
model cannot distinguish them; if same labels remain, include relation/alias in
`detail` in a later focused change rather than silently dropping one.

**Step 6: Run SQL completion tests**

Run:

```bash
cargo test --test sql_completion
```

Expected: all pass.

**Logical checkpoint:** flat query blocks correctly support joins, comma lists,
aliases, and qualified catalog paths.

## Task 5: Add Nested Query Scope Visibility

**Files:**
- Modify: `src/sql/completion.rs`
- Modify: `tests/sql_completion.rs`

**Step 1: Add failing nested-scope tests**

Cover these behaviors:

```rust
#[test]
fn subquery_does_not_leak_its_relations_to_the_outer_query() {
    // Cursor is in the outer WHERE after a completed subquery.
    // Outer relation columns are present; inner-only columns are absent.
}

#[test]
fn correlated_subquery_can_see_enclosing_relations() {
    // Cursor is inside an EXISTS subquery.
    // Inner relation columns and enclosing relation columns are present.
}

#[test]
fn sibling_subquery_relations_are_not_visible() {
    // Cursor is in the second sibling subquery.
    // Relations local to the first sibling are absent.
}
```

Keep CTE and derived-table output inference out of these tests. Their catalog
columns are unknown by design.

**Step 2: Run tests and verify failures**

Run:

```bash
cargo test --test sql_completion subquery_does_not_leak_its_relations_to_the_outer_query
cargo test --test sql_completion correlated_subquery_can_see_enclosing_relations
cargo test --test sql_completion sibling_subquery_relations_are_not_visible
```

Expected: the current flat relation list leaks at least one nested binding.

**Step 3: Build lexical query scopes from token depth**

Introduce a private scope representation:

```rust
#[derive(Clone, Debug, Default)]
struct QueryScope {
    depth: usize,
    start: usize,
    end: usize,
    parent: Option<usize>,
    bindings: Vec<RelationBinding>,
}
```

Identify a query scope when a `SELECT`, `WITH`, `UPDATE`, `INSERT`, or `DELETE`
statement begins at the statement root or inside parentheses. Close it when its
containing parenthesis closes or the statement ends. Locate the innermost scope
whose byte range contains the cursor.

Visible bindings are:

- bindings local to the cursor scope;
- bindings from each ancestor query scope for correlated references;
- never bindings from child or sibling scopes.

If malformed parentheses prevent exact closure, extend the innermost open scope
to the available statement end.

**Step 4: Make context detection scope-local**

Find the active clause from tokens in the cursor scope only. A `WHERE` or `FROM`
inside a completed child scope must not determine the outer cursor's context.

Return a single internal analysis object from one pass:

```rust
struct CompletionAnalysis {
    context: Context,
    bindings: Vec<RelationBinding>,
}
```

Use this object in both `complete` and `relation_ids_for_completion` so candidate
generation and metadata loading agree.

**Step 5: Run nested and full completion tests**

Run:

```bash
cargo test --test sql_completion subquery_does_not_leak_its_relations_to_the_outer_query
cargo test --test sql_completion correlated_subquery_can_see_enclosing_relations
cargo test --test sql_completion sibling_subquery_relations_are_not_visible
cargo test --test sql_completion
```

Expected: all pass.

**Logical checkpoint:** visible columns follow lexical query scope.

## Task 6: Verify Missing Metadata And App Loading Integration

**Files:**
- Modify: `tests/sql_completion.rs`
- Modify if signature changed: `src/app.rs:5384-5428`
- Modify if integration coverage is needed: `tests/app_flow.rs`

**Step 1: Add a completion test for unloaded relation children**

Create an index containing the referenced table but no column children:

```rust
#[test]
fn missing_columns_do_not_fall_back_to_global_relations() {
    let index = CompletionIndex::new(&relations_without_columns_fixture());
    let sql = "select * from sys_user where ";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    assert!(candidates.iter().all(|candidate| !matches!(
        candidate.kind,
        CompletionKind::Database
            | CompletionKind::Schema
            | CompletionKind::Table
            | CompletionKind::View
    )));
    assert!(candidates.iter().any(|candidate| {
        candidate.kind == CompletionKind::Keyword
    }));
}
```

**Step 2: Run the test**

Run:

```bash
cargo test --test sql_completion missing_columns_do_not_fall_back_to_global_relations
```

Expected after Tasks 1-5: pass. If it fails, fix only the fallback path; do not
invent synthetic columns.

**Step 3: Verify relation metadata discovery uses the same analysis**

Add or update a test asserting `relation_ids_for_completion` returns only
relations visible in the cursor scope and honors the active schema when duplicate
table names exist.

If the function now accepts `CompletionContext`, update `App::complete_now`:

1. construct `completion_context` before relation-ID discovery;
2. pass it to `relation_ids_for_completion`;
3. reuse the same value in `sql::complete`;
4. preserve existing `CatalogRequestIntent::Completion` behavior.

**Step 4: Add an app-level test only if pure-service coverage cannot prove loading**

Use existing app test patterns to confirm explicit completion schedules a
`CatalogTarget::RelationChildren` request for a visible unloaded relation and
does not schedule unrelated nested or ambiguous relations. Avoid broad UI
rendering changes.

**Step 5: Run focused app and completion tests**

Run:

```bash
cargo test --test sql_completion
cargo test --test app_flow completion
```

If `app_flow` has no matching completion test after implementation, run its
specific new test name instead of relying on a filter that executes zero tests.

Expected: all selected tests pass and relation-child loading remains non-blocking.

**Logical checkpoint:** filtering and metadata hydration agree on visible
relations.

## Task 7: Regression And Quality Verification

**Files:**
- Modify only if failures expose defects: `src/sql/completion.rs`,
  `tests/sql_completion.rs`, `src/app.rs`

**Step 1: Format changed Rust files**

Run:

```bash
cargo fmt --check
```

If it reports formatting differences, run `cargo fmt`, inspect the changed file
set, then rerun `cargo fmt --check`.

Expected: pass.

**Step 2: Run the SQL-focused test suites**

Run:

```bash
cargo test --test sql_completion
cargo test --test sql_scope
```

Expected: pass.

**Step 3: Run static analysis**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: pass with no warnings. Fix only warnings caused by this change; report
pre-existing unrelated warnings rather than modifying unrelated files.

**Step 4: Run the complete test suite**

Run:

```bash
cargo test --all-targets --all-features
```

Expected: pass.

**Step 5: Inspect the final diff**

Run:

```bash
git status --short
```

Confirm:

- unrelated existing worktree changes were not altered;
- filtering happens before sorting and truncation;
- no broad `General` fallback remains;
- completion and relation metadata discovery share context analysis;
- no compatibility branch restores mixed global candidates.

**Step 6: Record verification results**

In the implementation handoff, report exact commands run, their outcomes, and
any skipped checks or residual limitations. Do not claim unexecuted checks
passed.

## Manual Acceptance Scenarios

After automated verification, run LazyDB against a catalog with `sys_user` and
visually check:

1. `select * from sys_user` followed by `where ` shows `sys_user` columns and
   predicate keywords, not tables.
2. `select u from sys_user` shows matching `sys_user` columns, not unrelated
   `u...` tables and not statement-level `UPDATE`.
3. `select s. from sys_user s` shows only `sys_user` columns.
4. `select * from ` still shows databases, schemas, tables, and views.
5. `select * from schema.` still shows relations under that schema.
6. `select * from users u join roles r on ` shows columns from both relations.
7. Completion remains responsive with an empty expression prefix and a large
   catalog because global relation entries are not enumerated.

## Expected Changed Files

- `src/sql/completion.rs`
- `src/app.rs` only if relation-ID resolution needs `CompletionContext`
- `tests/sql_completion.rs`
- `tests/app_flow.rs` only if loading integration needs additional coverage
- `docs/plans/2026-08-31-sql-editor-contextual-completion-design.md`
- `docs/plans/2026-08-31-sql-editor-contextual-completion-implementation.md`

No UI rendering change is expected.
