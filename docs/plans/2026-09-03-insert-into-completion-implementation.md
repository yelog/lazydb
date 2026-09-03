# SQL Editor INSERT INTO Completion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make SQL Editor suggest `INTO` after `INSERT`, including immediately after the trailing space, without offering `INTO` at a statement start or regressing relation completion after `INSERT INTO`.

**Architecture:** Extend the existing tolerant token-based completion state machine with a private `Context::Insert` transition. Keep candidate filtering and ranking unchanged except for the new context-specific `INTO` keyword, and extend the existing edit-trigger predicate so typing the space after `INSERT` schedules completion.

**Tech Stack:** Rust 2024, existing SQL completion tokenizer/state machine, Cargo test, rustfmt, Clippy

---

### Task 1: Specify INSERT keyword-transition behavior

**Files:**
- Modify: `tests/sql_completion.rs:10-12,273-302`
- Test: `tests/sql_completion.rs`

**Step 1: Import the completion-trigger predicate**

Add `should_offer_completion` to the existing `lazydb::sql` imports:

```rust
sql::{
    CompletionContext, CompletionIndex, CompletionKind, SqlDialect, complete, quote_identifier,
    should_offer_completion,
},
```

Keep the import formatting produced by rustfmt; do not alter unrelated imports.

**Step 2: Write a failing test for the expected keyword transition**

Add this test beside `statement_and_expression_keywords_are_contextual`:

```rust
#[test]
fn insert_completion_offers_only_into_keyword() {
    let index = CompletionIndex::new(&contextual_fixture());

    for sql in ["insert ", "insert i"] {
        let candidates = complete(
            sql,
            sql.len(),
            SqlDialect::Postgres,
            &index,
            CompletionContext::default(),
        );

        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.kind == CompletionKind::Keyword)
                .map(|candidate| candidate.label.as_str())
                .collect::<Vec<_>>(),
            vec!["INTO"],
            "unexpected keyword candidates for {sql}: {candidates:?}"
        );
    }
}
```

This test deliberately uses a populated catalog. It proves the result comes from syntax context rather than from an empty candidate index.

**Step 3: Write a regression test for statement-start isolation and relation continuation**

Add:

```rust
#[test]
fn insert_context_does_not_leak_into_statement_or_relation_completion() {
    let index = CompletionIndex::new(&fixture());

    let statement = complete(
        "i",
        1,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert_eq!(
        statement.first().map(|candidate| candidate.label.as_str()),
        Some("INSERT")
    );
    assert!(statement.iter().all(|candidate| candidate.label != "INTO"));

    let relation_sql = "insert into u";
    let relation = complete(
        relation_sql,
        relation_sql.len(),
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );
    assert!(relation.iter().any(|candidate| {
        candidate.kind == CompletionKind::Table && candidate.label == "users"
    }));
}
```

This locks down both sides of the new state: `INTO` is not a statement-start keyword, and the existing `INTO -> Relation` transition still produces table candidates.

**Step 4: Write a failing test for automatic completion triggering**

Add:

```rust
#[test]
fn insert_space_triggers_completion() {
    assert!(should_offer_completion("insert ", "insert ".len()));
    assert!(should_offer_completion("INSERT ", "INSERT ".len()));
}
```

The uppercase assertion preserves the current case-insensitive trigger contract.

**Step 5: Run the focused tests and verify the failure reasons**

Run:

```bash
cargo test --test sql_completion insert_completion_offers_only_into_keyword -- --exact
cargo test --test sql_completion insert_context_does_not_leak_into_statement_or_relation_completion -- --exact
cargo test --test sql_completion insert_space_triggers_completion -- --exact
```

Expected before implementation:

- `insert_completion_offers_only_into_keyword` fails because `insert i` yields `INSERT`, while `insert ` yields statement-start keywords rather than only `INTO`.
- `insert_context_does_not_leak_into_statement_or_relation_completion` should already pass and acts as the regression baseline.
- `insert_space_triggers_completion` fails because `should_offer_completion` does not recognize `insert` before whitespace.

Do not weaken the assertions to match the current behavior.

### Task 2: Add an INSERT-specific completion context

**Files:**
- Modify: `src/sql/completion.rs:162-169,273-280,323-329,527-547,550-580,1127-1155`
- Test: `tests/sql_completion.rs`

**Step 1: Add the private context variant**

Extend `Context` with an `Insert` variant between `Statement` and `Relation`:

```rust
enum Context {
    Statement,
    Insert,
    Relation,
    Expression(ExpressionContext),
    Qualifier,
    Routine,
}
```

Keep this internal. Do not add a public completion kind or modify popup rendering because `INTO` remains a normal `CompletionKind::Keyword` candidate.

**Step 2: Transition into the new context after a complete INSERT token**

In `context_at`, add the transition before relation-introducing keywords:

```rust
"insert" => Context::Insert,
"from" | "join" | "update" | "into" => Context::Relation,
```

Continue analyzing only tokens ending before the replacement range. For `insert i`, this means `insert` determines context while the incomplete `i` remains the filtering prefix.

**Step 3: Restrict catalog candidates in INSERT context**

Update `catalog_kind_allowed` so `Insert` rejects catalog entries just like `Statement`:

```rust
Context::Statement | Context::Insert => false,
```

Do not expose table candidates until `INTO` has moved the state to `Context::Relation`.

**Step 4: Provide only the context-valid keyword**

Add this arm to `keywords` immediately after the statement arm:

```rust
Context::Insert => &["INTO"],
```

Do not add `INTO` to the statement keyword arrays. That would make `INTO` appear at a fresh statement start and recreate a semantic filtering problem.

**Step 5: Give the INSERT transition normal syntax priority**

Update the keyword score match in `complete`:

```rust
context: match (context, projection_complete, *keyword) {
    (Context::Expression(ExpressionContext::Projection), true, "FROM") => 4,
    (Context::Statement | Context::Insert, _, _) => 4,
    (Context::Expression(_), _, _) => 2,
    (Context::Relation | Context::Routine, _, _) => 1,
    (Context::Qualifier, _, _) => 0,
},
```

The new context currently has one keyword, but assigning the same priority as other structural transitions keeps scoring explicit and future-proof without adding a new score field.

**Step 6: Run the keyword-transition tests**

Run:

```bash
cargo test --test sql_completion insert_completion_offers_only_into_keyword -- --exact
cargo test --test sql_completion insert_context_does_not_leak_into_statement_or_relation_completion -- --exact
```

Expected: both tests pass. Specifically, `insert ` and `insert i` expose `INTO`, a standalone `i` still ranks `INSERT` first, and `insert into u` still exposes table `users`.

**Step 7: Commit the completed context change**

After reviewing the diff and only if commits are part of the execution request:

```bash
git add src/sql/completion.rs tests/sql_completion.rs
git commit -m "fix(sql): suggest into after insert"
```

### Task 3: Trigger completion immediately after INSERT whitespace

**Files:**
- Modify: `src/sql/completion.rs:460-482`
- Test: `tests/sql_completion.rs`

**Step 1: Extend the existing trigger allowlist**

Add `insert` to the whitespace-trigger keyword list in `should_offer_completion`:

```rust
return [
    "from", "join", "update", "insert", "into", "select", "where",
]
.iter()
.any(|keyword| before.ends_with(keyword));
```

Retain the existing lowercasing and cursor bounds behavior. Do not refactor the trigger parser in this focused fix; completion candidate generation remains responsible for semantic correctness.

**Step 2: Run the trigger test**

Run:

```bash
cargo test --test sql_completion insert_space_triggers_completion -- --exact
```

Expected: PASS for lowercase and uppercase `INSERT` followed by a space.

**Step 3: Run all newly added regression tests together**

Run:

```bash
cargo test --test sql_completion insert_ -- --nocapture
```

Expected: all tests whose names begin with `insert_` pass.

**Step 4: Commit the trigger change**

After reviewing the diff and only if commits are part of the execution request:

```bash
git add src/sql/completion.rs tests/sql_completion.rs
git commit -m "fix(sql): trigger completion after insert"
```

### Task 4: Verify the full completion subsystem

**Files:**
- Verify: `src/sql/completion.rs`
- Verify: `tests/sql_completion.rs`

**Step 1: Format the changed Rust files**

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

Expected: all completion tests pass, including existing statement keyword ranking, projection, predicate, qualifier, relation, and catalog matching coverage.

**Step 3: Run the full test suite**

Run:

```bash
cargo test --all-features
```

Expected: all project tests pass.

**Step 4: Run lint and whitespace validation**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

Expected: Clippy reports no warnings and Git reports no whitespace errors.

**Step 5: Perform a manual SQL Editor smoke test**

Start LazyDB using the project's normal development command and verify:

1. Typing `i` at an empty statement offers `INSERT`, not `INTO`.
2. Typing `insert ` immediately opens completion with `INTO` selected.
3. Typing `insert i` shows `INTO` and does not show `INSERT`.
4. Accepting `INTO`, typing a space, and then a table prefix offers matching tables.
5. Repeat the flow with uppercase `INSERT` to confirm case-insensitive context handling.

No README or keybinding documentation change is required because this fixes existing SQL completion behavior without changing commands, keys, configuration, or public APIs.
