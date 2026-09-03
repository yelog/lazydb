# SQL Editor INSERT Column List Completion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make SQL Editor suggest only the target table's columns while the cursor is inside an `INSERT INTO <relation> (...)` target-column list, without leaking statement keywords or breaking nested-query completion.

**Architecture:** Extend the existing tolerant token-based completion state machine with a private `InsertColumns` context. Detect that context by relating the active opening parenthesis to an `INSERT INTO` target binding in its parent scope, then source candidates directly from that resolved relation's children instead of scanning the global catalog. Reuse the existing relation resolution and lazy relation-child loading paths so active database/schema disambiguation and asynchronous metadata refresh remain unchanged.

**Tech Stack:** Rust 2024, existing `CompletionToken` lexer and scope model, existing `CompletionIndex` catalog metadata, Cargo integration tests, rustfmt, Clippy

---

## Constraints

- Preserve the public signatures of `sql::complete`, `sql::relation_ids_for_completion`, `CompletionCandidate`, and `CompletionIndex`.
- Preserve the existing ten-candidate limit, identifier quoting, compact matching, terminal sanitization, and popup lifecycle.
- Keep completion tolerant of incomplete SQL. Do not replace the current lexer with strict `sqlparser` AST parsing because the input may have no closing parenthesis or `VALUES` clause yet.
- Do not treat every parenthesized expression as an INSERT column list. Function calls, subqueries, `IN (...)`, and `VALUES (...)` must retain their existing behavior.
- Do not make columns globally valid in `Context::Statement`; target-table ownership must be proven before a column candidate is admitted.
- Do not add a new dependency or modify UI rendering, keybindings, catalog models, or database drivers.
- Excluding already-entered target columns is a follow-up enhancement, not part of this correctness fix.
- Do not modify or revert unrelated worktree changes. Do not commit unless the execution request explicitly includes commits; commit commands below mark logical checkpoints only.

## Target Behavior

The following cursor positions must use target-column completion (`|` denotes the cursor):

```sql
INSERT INTO sys_user(|
INSERT INTO sys_user (update_|
INSERT INTO sys_user (id, user_|
INSERT INTO public.sys_user (|
INSERT INTO "public"."sys_user" (|
```

At those positions:

- candidates are `CompletionKind::Column` only;
- candidates belong to the resolved INSERT target relation only;
- statement keywords such as `DELETE`, `INSERT`, `SELECT`, `UPDATE`, and `WITH` are absent;
- active database/schema still resolves duplicate unqualified relation names;
- column label, insertion text, native-type detail, prefix matching, and quoting follow existing behavior.

The following positions must not use target-column completion:

```sql
SELECT count(|
SELECT * FROM (|
SELECT * FROM users WHERE id IN (|
INSERT INTO sys_user (id) VALUES (|
INSERT INTO sys_user VALUES (|
INSERT INTO sys_user DEFAULT VALUES
INSERT INTO sys_user SELECT (|
INSERT INTO sys_user SET value = (|
```

## Task 1: Lock Down the Reported Behavior

**Files:**
- Modify: `tests/sql_completion.rs:198-365`
- Test: `tests/sql_completion.rs`

**Step 1: Add a helper for candidate labels**

Near the existing completion fixtures, add a small test-only helper to keep assertions readable:

```rust
fn labels(candidates: &[lazydb::sql::CompletionCandidate]) -> Vec<&str> {
    candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect()
}
```

If importing `CompletionCandidate` produces cleaner code, add it to the existing `lazydb::sql` import and use `&[CompletionCandidate]`. Do not introduce a production helper solely for tests.

**Step 2: Write the failing regression test for an empty target-column list**

Add this test beside the existing INSERT completion tests:

```rust
#[test]
fn insert_column_list_only_offers_target_columns() {
    let index = CompletionIndex::new(&contextual_fixture());
    let sql = "insert into sys_user(";
    let candidates = complete(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );

    assert_eq!(
        labels(&candidates),
        [
            "update_time",
            "update_user",
            "update_user_phone",
            "user_type",
            "username",
        ]
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind == CompletionKind::Column)
    );
}
```

The exact label assertion proves that unrelated relation columns and statement keywords are not merely ranked lower; they are excluded from the candidate set.

**Step 3: Run the focused test and verify the current failure**

Run:

```bash
cargo test --test sql_completion insert_column_list_only_offers_target_columns -- --exact
```

Expected before implementation: FAIL. The actual labels are statement-level keywords, beginning with `DELETE`, because the active parenthesis scope is classified as `Context::Statement`.

**Step 4: Add prefix and comma-continuation tests**

Add:

```rust
#[test]
fn insert_column_list_filters_prefix_and_continues_after_comma() {
    let index = CompletionIndex::new(&contextual_fixture());
    let context = CompletionContext {
        database: Some("app"),
        schema: Some("public"),
    };

    let prefix_sql = "insert into sys_user(update_u";
    let prefix = complete(
        prefix_sql,
        prefix_sql.len(),
        SqlDialect::Postgres,
        &index,
        context,
    );
    assert_eq!(labels(&prefix), ["update_user", "update_user_phone"]);

    let comma_sql = "insert into sys_user(update_time, user_";
    let after_comma = complete(
        comma_sql,
        comma_sql.len(),
        SqlDialect::Postgres,
        &index,
        context,
    );
    assert_eq!(labels(&after_comma), ["user_type", "username"]);
}
```

**Step 5: Run both INSERT column-list tests**

Run:

```bash
cargo test --test sql_completion insert_column_list_ -- --nocapture
```

Expected before implementation: both tests fail because the current scope has no INSERT-specific semantic state.

## Task 2: Detect the INSERT Target-Column Scope

**Files:**
- Modify: `src/sql/completion.rs:162-208,210-246,553-594,827-905`
- Test: `tests/sql_completion.rs`

**Step 1: Add the private completion context**

Extend the private `Context` enum:

```rust
enum Context {
    Statement,
    Insert,
    InsertColumns,
    Relation,
    Expression(ExpressionContext),
    Qualifier,
    Routine,
}
```

Do not add a public `CompletionKind`: displayed entries remain ordinary `CompletionKind::Column` candidates.

**Step 2: Add a scope-aware target detector**

First extend `is_relation_boundary` with the clause words that can immediately follow an INSERT target but must never be consumed as an implicit alias:

```rust
"values" | "select" | "default" | "overriding" | "set"
```

This correction is necessary because `relation_binding_at` currently treats an unrecognized word after a relation as an alias. Without these boundaries, `INSERT INTO sys_user VALUES (` could be parsed as relation `sys_user`, alias `VALUES`, followed by `(` and would be falsely recognized as a target-column list. Keep normal aliases supported, including PostgreSQL's `INSERT INTO sys_user AS target (...)` form.

Then add a private helper near `context_at`/`relation_binding_at`:

```rust
fn insert_column_target_at(
    tokens: &[CompletionToken],
    cursor: usize,
    current_scope: Option<usize>,
) -> Option<RelationBinding> {
    let opening_start = current_scope?;
    let opening_index = tokens.iter().position(|token| {
        token.start == opening_start
            && token.start < cursor
            && token.kind == CompletionTokenKind::LeftParen
    })?;
    let parent_scope = tokens[opening_index].scope_start;

    let insert_index = tokens[..opening_index].iter().rposition(|token| {
        token.scope_start == parent_scope
            && token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case("insert"))
    })?;
    let into_index = tokens[insert_index + 1..opening_index]
        .iter()
        .position(|token| {
            token.scope_start == parent_scope
                && token_word(Some(token)).is_some_and(|word| word.eq_ignore_ascii_case("into"))
        })?
        + insert_index
        + 1;
    let (binding, next) = relation_binding_at(tokens, into_index + 1)?;

    (binding.scope_start == parent_scope && next == opening_index).then_some(binding)
}
```

The implementation may use iterator helpers with equivalent behavior, but it must preserve these invariants:

- identify the exact opening parenthesis that owns the cursor's active scope;
- inspect only the opening parenthesis's parent scope;
- require `INSERT` and `INTO` in that same parent scope;
- parse the relation with the existing `relation_binding_at` logic so qualified and quoted names continue to work;
- require the parsed relation binding to end exactly at this opening parenthesis;
- return `None` for a later `VALUES (` parenthesis because the target relation binding ends at the earlier column-list parenthesis;
- return `None` for `INSERT INTO relation VALUES (`, `DEFAULT VALUES`, `INSERT ... SELECT`, PostgreSQL `OVERRIDING`, and MySQL `INSERT ... SET` continuations because clause words are relation boundaries, not aliases;
- return `None` for unrelated or nested parentheses.

Do not infer this state with a regex over raw SQL. The existing lexer already protects strings, comments, quoted identifiers, and nesting.

**Step 3: Select `InsertColumns` before generic context classification**

In `complete`, store the current scope once and use the detector:

```rust
let active_scopes = active_scope_starts(&tokens, statement_cursor);
let current_scope = active_scopes.last().copied().flatten();
let insert_column_target =
    insert_column_target_at(&tokens, statement_cursor, current_scope);
let context = if insert_column_target.is_some() {
    Context::InsertColumns
} else {
    context_at(&tokens, statement_cursor, current_scope)
};
```

Pass `current_scope` to `projection_is_complete` instead of recalculating it. Keep `visible_relation_bindings` unchanged because enclosing relations are still required for normal expression/subquery completion.

**Step 4: Compile to expose all exhaustive-match updates**

Run:

```bash
cargo test --test sql_completion insert_column_list_only_offers_target_columns --no-run
```

Expected at this intermediate point: compilation fails at exhaustive matches in candidate filtering, scoring, and `keywords`. Use those compiler errors as the checklist for Task 3; do not add wildcard arms that could hide missing context policy.

**Step 5: Logical checkpoint**

At this point detection exists, but no candidate policy has been assigned. Do not commit this non-compiling intermediate state.

## Task 3: Restrict Candidates to the Resolved Target Relation

**Files:**
- Modify: `src/sql/completion.rs:210-345,487-550,742-813,1131-1160`
- Test: `tests/sql_completion.rs`

**Step 1: Resolve the INSERT target relation through the existing resolver**

After obtaining `insert_column_target`, resolve it with the same database/schema-aware helper used by normal relation bindings:

```rust
let insert_target_relations = insert_column_target
    .as_ref()
    .into_iter()
    .flat_map(|binding| relation_ids(index, binding, completion_context))
    .collect::<HashSet<_>>();
```

Using `relation_ids` is required. Do not introduce a second relation-name resolver, because that would diverge on active schema selection and qualified names.

**Step 2: Add a target-child candidate index helper**

Add a private helper beside `candidate_indices`:

```rust
fn relation_child_candidate_indices(
    index: &CompletionIndex,
    relations: &HashSet<CatalogId>,
) -> Vec<usize> {
    relations
        .iter()
        .flat_map(|relation| index.children.get(relation).into_iter().flatten().copied())
        .collect()
}
```

If deterministic deduplication is needed, retain the first occurrence with a local `HashSet<usize>`. Do not clone `CatalogEntry` values.

**Step 3: Choose the candidate source by semantic context**

Replace the unconditional `qualified_candidate_indices` call with:

```rust
let candidate_indexes = if context == Context::InsertColumns {
    if qualifiers.is_empty() {
        relation_child_candidate_indices(index, &insert_target_relations)
    } else {
        Vec::new()
    }
} else {
    qualified_candidate_indices(
        index,
        &qualifiers,
        &folded_prefix,
        &bindings,
        completion_context,
    )
};
```

Target columns in an INSERT column list are unqualified identifiers. Returning no candidates for `sys_user.column` avoids suggesting syntactically invalid target-column forms.

**Step 4: Add strict kind filtering for the new context**

Update `catalog_kind_allowed`:

```rust
Context::Statement | Context::Insert => false,
Context::InsertColumns => kind == CompletionKind::Column,
```

The direct child source enforces ownership; this allowlist independently enforces catalog kind. Keep both checks so future catalog-child kinds cannot leak into this context.

**Step 5: Give target columns the normal column context score**

Update candidate scoring:

```rust
(Context::InsertColumns, CompletionKind::Column)
| (Context::Relation, CompletionKind::Table | CompletionKind::View)
| (Context::Qualifier, CompletionKind::Column)
| (Context::Expression(_), CompletionKind::Column)
| (Context::Routine, CompletionKind::Function | CompletionKind::Procedure) => 3,
```

Do not give target columns a score above other exact/prefix matching rules. There are no competing candidate kinds in this context.

**Step 6: Disable keywords in the new context**

Update `keywords`:

```rust
Context::InsertColumns => &[],
```

Update the keyword score match exhaustively, even though this context supplies no keywords:

```rust
(Context::Qualifier | Context::InsertColumns, _, _) => 0,
```

Do not remove statement keywords globally or merely lower their score.

**Step 7: Run the focused tests**

Run:

```bash
cargo test --test sql_completion insert_column_list_ -- --nocapture
```

Expected: both Task 1 tests pass. Empty and prefixed target lists contain only `sys_user` columns; no statement keywords or columns from `user_agreement_accept`/`unit_mtmm_capacity` appear.

**Step 8: Run existing INSERT transition tests**

Run:

```bash
cargo test --test sql_completion insert_completion_offers_only_into_keyword -- --exact
cargo test --test sql_completion insert_context_does_not_leak_into_statement_or_relation_completion -- --exact
cargo test --test sql_completion insert_space_triggers_completion -- --exact
```

Expected: all pass. The existing `INSERT -> INTO -> relation` flow remains unchanged outside the target-column parenthesis.

**Step 9: Optional logical commit**

Only if commits were explicitly requested:

```bash
git add src/sql/completion.rs tests/sql_completion.rs
git commit -m "fix(sql): complete insert target columns"
```

## Task 4: Preserve Lazy Column Metadata Loading

**Files:**
- Modify: `tests/sql_completion.rs`
- Verify: `src/sql/completion.rs:348-364,827-929`
- Verify: `src/app.rs:7995-8090,9081-9085`

**Step 1: Add a regression test for target relation discovery**

Add a test proving `relation_ids_for_completion` still discovers the target while the cursor is inside its column-list scope. Import `relation_ids_for_completion` from `lazydb::sql` and add:

```rust
#[test]
fn insert_column_list_requests_target_relation_columns() {
    let entries = contextual_fixture();
    let expected = entries
        .iter()
        .find(|entry| {
            entry.kind == CatalogKind::Table && entry.qualified_name.object == "sys_user"
        })
        .unwrap()
        .id
        .clone();
    let index = CompletionIndex::new(&entries);
    let sql = "insert into sys_user(";

    let relations = relation_ids_for_completion(
        sql,
        sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext {
            database: Some("app"),
            schema: Some("public"),
        },
    );

    assert_eq!(relations, [expected]);
}
```

If relation ordering is not guaranteed because the production function intentionally collects through a `HashSet`, assert length and membership rather than exact vector equality.

**Step 2: Run the relation discovery test**

Run:

```bash
cargo test --test sql_completion insert_column_list_requests_target_relation_columns -- --exact
```

Expected: PASS without production changes. `visible_relation_bindings` includes the outer `INTO sys_user` binding because `active_scope_starts` contains both the outer and current parenthesis scopes.

**Step 3: Preserve the application refresh path**

Confirm no change is required in `App::complete_now`:

- `relation_ids_for_completion` returns `sys_user`;
- unloaded `CatalogTarget::RelationChildren` triggers a completion catalog request;
- the first completion call may temporarily have no column candidates;
- `apply_catalog_page` calls `complete_now()` after relation children arrive;
- the refreshed popup then contains the target columns.

Do not duplicate metadata loading inside `complete()` and do not synchronously query a database from the pure completion service.

**Step 4: Add an app-level test only if the existing refresh contract fails**

If Step 2 passes and existing catalog completion tests cover relation-child refresh, do not add a large app fixture. If implementation changes unexpectedly break this flow, add a focused app reducer test that asserts a `CatalogTarget::RelationChildren` command for `insert into sys_user(` and popup refresh after the page is applied.

## Task 5: Add Scope, Schema, and Dialect Regression Coverage

**Files:**
- Modify: `tests/sql_completion.rs`
- Test: `tests/sql_completion.rs`

**Step 1: Add negative parenthesis cases**

Add a table-driven test:

```rust
#[test]
fn unrelated_parentheses_are_not_insert_column_lists() {
    let index = CompletionIndex::new(&contextual_fixture());

    for sql in [
        "select count(",
        "select * from (",
        "select * from sys_user where username in (",
        "insert into sys_user (username) values (",
        "insert into sys_user values (",
        "insert into sys_user default values",
        "insert into sys_user select (",
        "insert into sys_user set username = (",
    ] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext {
                database: Some("app"),
                schema: Some("public"),
            },
        );

        assert!(
            candidates.iter().any(|candidate| candidate.kind != CompletionKind::Column)
                || candidates.is_empty(),
            "unexpected INSERT target columns for {sql}: {candidates:?}"
        );
    }
}
```

For the direct INSERT continuations (`VALUES`, `DEFAULT VALUES`, `SELECT`, and `SET`), assert specifically that the result is not the full target-column-only set. For predicate/subquery cases, avoid a blanket “no columns” assertion because normal visible-column expression completion can be valid. This test must fail if any clause word is accidentally consumed as a relation alias.

**Step 2: Add duplicate-schema target resolution coverage**

Create a compact fixture with `public.events(public_value)` and `audit.events(audit_value)`, following `active_schema_resolves_duplicate_unqualified_relations`. Complete:

```sql
insert into events(
```

with `CompletionContext { database: Some("app"), schema: Some("audit") }` and assert:

```rust
assert_eq!(labels(&candidates), ["audit_value"]);
```

This proves the implementation reused `relation_ids` rather than selecting every table with the same object name.

**Step 3: Add qualified and quoted relation coverage**

Add a table-driven test using the existing fixture:

```rust
for (sql, dialect) in [
    ("insert into public.sys_user(", SqlDialect::Postgres),
    ("insert into \"public\".\"sys_user\"(", SqlDialect::Postgres),
    ("insert into `app`.`sys_user`(", SqlDialect::MySql),
] {
    // Assert that `username` is present and every candidate is a column.
}
```

For SQL Server, add a dedicated fixture whose qualified name matches the expected database/schema path, then cover:

```sql
insert into [dbo].[sys_user](
```

Do not force all dialect cases through a fixture whose database/schema semantics do not match that dialect.

**Step 4: Run all new INSERT column tests**

Run:

```bash
cargo test --test sql_completion insert_column -- --nocapture
cargo test --test sql_completion unrelated_parentheses_are_not_insert_column_lists -- --exact
```

Expected: all pass.

**Step 5: Run existing nested-scope tests explicitly**

Run:

```bash
cargo test --test sql_completion correlated_subquery_can_see_enclosing_relations -- --exact
cargo test --test sql_completion subquery_does_not_leak_its_relations_to_the_outer_query -- --exact
cargo test --test sql_completion sibling_subquery_relations_are_not_visible -- --exact
```

Expected: all pass. The change identifies one semantic parenthesis case without altering global parenthesis scope tracking.

**Step 6: Optional logical commit**

Only if commits were explicitly requested:

```bash
git add tests/sql_completion.rs
git commit -m "test(sql): cover insert column contexts"
```

## Task 6: Verify the Completion Subsystem and Full Project

**Files:**
- Verify: `src/sql/completion.rs`
- Verify: `tests/sql_completion.rs`
- Verify: `src/app.rs`

**Step 1: Format and check formatting**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
```

Expected: both commands exit successfully and the check reports no formatting diff.

**Step 2: Run the complete SQL completion suite**

Run:

```bash
cargo test --test sql_completion --all-features
```

Expected: all existing and newly added completion tests pass.

**Step 3: Run Clippy for all targets**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: no warnings. In particular, do not silence unnecessary cloning, needless collection, or exhaustive-match warnings with broad attributes.

**Step 4: Run the full test suite**

Run:

```bash
cargo test --all-features
```

Expected: all project tests pass.

**Step 5: Check the final diff**

Run:

```bash
git diff --check
git diff -- src/sql/completion.rs tests/sql_completion.rs
```

Expected: no whitespace errors. The functional diff is limited to the completion context/detector/candidate source and targeted tests.

**Step 6: Perform a manual SQL Editor smoke test**

Start LazyDB using the project's normal development command with a connection whose `sys_user` columns are available, then verify:

1. Typing `insert ` still offers only `INTO`.
2. Typing `insert into sys_u` still offers the `sys_user` table.
3. Typing `insert into sys_user(` opens or refreshes completion with only `sys_user` columns.
4. Typing `update_` inside the list narrows candidates to matching `sys_user` columns.
5. Typing a comma and another prefix continues target-column completion.
6. `insert into sys_user(username) values (` does not show the target column list again.
7. `insert into sys_user values (` does not mistake `values` for an alias or show target columns.
8. `select count(` and `select * from (` do not show INSERT target columns.
9. If relation children were initially unloaded, the popup refreshes after metadata loading and then shows the columns.

**Step 7: Final acceptance criteria**

The implementation is complete only when:

- `INSERT INTO sys_user(` produces target-table columns instead of statement keywords;
- no candidate belongs to another relation;
- duplicate table names resolve through active database/schema context;
- quoted and qualified relation names work for the covered dialects;
- `VALUES (`, function calls, predicates, and subqueries are not misclassified;
- lazy relation-child loading still refreshes completion;
- all focused tests, the full completion suite, Clippy, formatting, and the full project suite pass.

No README, configuration, or keybinding documentation change is required because this corrects existing SQL completion semantics without changing a public command or user-configurable behavior.
