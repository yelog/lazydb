# SELECT Query Row Limit Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bound eligible ad-hoc read queries to the existing 500-row preview limit before database execution.

**Architecture:** Add a small SQL transformation at the application execution boundary. Reuse the parser/classifier already used by derived result filtering to identify one read-only SELECT, preserve queries that already have a row limit, and otherwise wrap them in a dialect-compatible derived query with `LIMIT 500`. Keep non-query execution unchanged and store the transformed SQL only as the executed request while retaining the existing result model and grid behavior.

**Tech Stack:** Rust, `sqlparser`, SQLx, SQLite/MySQL/PostgreSQL adapters, existing Rust unit/integration tests.

---

### Task 1: Define bounded-query transformation behavior with tests

**Files:**
- Modify: `src/sql/derived_result.rs`
- Test: `src/sql/derived_result.rs`

**Step 1: Write focused failing tests**

Add tests for:

- a plain `SELECT * FROM users` receiving `LIMIT 500`;
- an existing `LIMIT 20` remaining unchanged and not receiving a second limit;
- trailing semicolon handling;
- non-query, multi-statement, and unsafe statements remaining ineligible;
- SQLite, MySQL, and PostgreSQL parser dialects where the existing parser supports them.

**Step 2: Run the focused tests**

Run: `cargo test sql::derived_result --lib`

Expected: the new tests fail because the bounded execution transformation does not yet exist.

**Step 3: Implement the minimal transformation**

Extend the existing derived-query helper or add a sibling helper that:

- accepts SQL source and `SqlDialect`;
- validates it is one read-only SELECT statement;
- detects an existing `LIMIT`/equivalent row cap from the parsed AST;
- strips only a trailing semicolon before wrapping;
- returns `SELECT * FROM (<source>) AS __lazydb_query LIMIT 500` for eligible uncapped SELECTs;
- returns the original normalized query for already-capped SELECTs;
- returns `None`/an explicit ineligible result for all other SQL.

Use `RELATION_PREVIEW_LIMIT` rather than another literal constant.

**Step 4: Run the focused tests**

Run: `cargo test sql::derived_result --lib`

Expected: PASS.

### Task 2: Apply the transformation at the execution boundary

**Files:**
- Modify: `src/app.rs` and/or `src/sql/execution.rs` at query dispatch
- Modify: `src/db/mod.rs` only if the shared execution boundary is the correct layer
- Test: `tests/sql_execution.rs` or the closest existing execution-flow test module

**Step 1: Trace the final SQL dispatch and add a failing integration test**

Use a mock or SQLite execution path to verify a large SELECT returns no more than
`RELATION_PREVIEW_LIMIT` rows, while an UPDATE/INSERT path remains unaffected.

**Step 2: Run the integration test**

Run: `cargo test --test sql_execution`

Expected: the SELECT limit assertion fails before wiring the transformation into execution.

**Step 3: Wire bounded SQL into execution**

Transform only the SQL sent through ordinary ad-hoc query execution. Do not alter
relation preview SQL, mutation SQL, transaction control, or output-only execution.
Ensure the dialect comes from the active execution target/profile and that errors
from parsing do not make unrelated SQL execution fail.

**Step 4: Preserve observability**

Keep `QueryOutcome.stats.row_count` based on rows actually loaded. If the result
has exactly the cap number of rows, add concise output/result feedback indicating
that the result is capped, without changing grid scrolling semantics.

**Step 5: Run execution tests**

Run: `cargo test --test sql_execution`

Expected: PASS, including the bounded SELECT and unchanged mutation cases.

### Task 3: Verify all database adapters and UI behavior

**Files:**
- Modify: `src/db/query.rs` only if shared limit metadata is needed
- Modify: `src/ui/mod.rs` or `src/ui/relation.rs` only for the cap notice
- Test: existing adapter tests and UI render tests as appropriate

**Step 1: Run adapter and unit tests**

Run: `cargo test --lib`

Expected: PASS.

**Step 2: Run the full test suite**

Run: `cargo test`

Expected: PASS.

**Step 3: Run formatting and lint checks**

Run: `cargo fmt --check`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: both commands pass.

**Step 4: Inspect the final diff**

Run: `git diff -- src/sql/derived_result.rs src/app.rs src/sql/execution.rs src/db/mod.rs src/db/query.rs src/ui/mod.rs src/ui/relation.rs tests`

Confirm unrelated existing worktree changes are untouched and no query path
accidentally applies the limit to mutations or multi-statement SQL.
