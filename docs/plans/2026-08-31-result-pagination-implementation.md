# Result Pagination Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace fixed 500-row relation and eligible SQL result limits with shared first/previous/next/last pagination, selectable page sizes, lower-bound totals, and on-demand exact counts.

**Architecture:** Introduce a pure pagination model shared by relation and SQL tabs, then make SQL builders produce one-row-lookahead page queries and exact count queries. Carry typed page requests and metadata through App, Runtime, and database adapters, preserving existing request generations, cancellation, transaction routing, and previous-result loading states. Render one reusable pagination bar on both result surfaces.

**Tech Stack:** Rust, Ratatui, Crossterm, SQLx, `sqlparser`, SQLite, PostgreSQL, MySQL, existing reducer/runtime/UI integration tests.

---

## Preconditions

- Execute this plan in a dedicated worktree.
- Preserve unrelated changes currently present in `src/ui/query_bar.rs`, `tests/ui_render.rs`, and contextual-key-hints plan files unless they have already been integrated before execution.
- Read `docs/plans/2026-08-31-result-pagination-design.md` before implementation.
- Use TDD for each task and keep commits limited to the files listed for that task.
- Do not remove the safety cap until the replacement page request is wired through the corresponding execution path.

### Task 1: Add The Shared Pagination Model

**Files:**
- Create: `src/model/pagination.rs`
- Modify: `src/model/mod.rs`
- Test: `src/model/pagination.rs`

**Step 1: Write failing page-size tests**

Define tests requiring a closed page-size type with exactly these values:

```rust
assert_eq!(PageSize::ALL, [
    PageSize::Ten,
    PageSize::OneHundred,
    PageSize::TwoHundredFifty,
    PageSize::FiveHundred,
    PageSize::OneThousand,
]);
assert_eq!(PageSize::default().get(), 500);
assert_eq!(PageSize::OneThousand.lookahead_limit(), 1001);
```

Also require `TryFrom<usize>` to reject values outside the five supported choices.

**Step 2: Write failing pagination-state tests**

Cover these exact state transitions:

```rust
let first = ResultPagination::from_page(PageRequest::first(PageSize::FiveHundred), 501);
assert_eq!(first.visible_rows, 500);
assert!(first.has_next);
assert_eq!(first.total, TotalRows::LowerBound(501));
assert_eq!(first.range(), Some(1..=500));

let last = ResultPagination::from_page(
    PageRequest::at(PageSize::FiveHundred, 1000),
    234,
);
assert_eq!(last.total, TotalRows::Exact(1234));
assert_eq!(last.range(), Some(1001..=1234));

let empty = ResultPagination::from_page(PageRequest::first(PageSize::FiveHundred), 0);
assert_eq!(empty.total, TotalRows::Exact(0));
assert_eq!(empty.range(), None);
```

Test `last_offset` for totals `0`, `1`, `500`, `501`, `1000`, and `1001`.

**Step 3: Run the focused tests**

Run: `cargo test model::pagination --lib`

Expected: FAIL because `model::pagination` does not exist.

**Step 4: Implement the minimal model**

Add:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PageSize {
    Ten,
    OneHundred,
    TwoHundredFifty,
    #[default]
    FiveHundred,
    OneThousand,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PageRequest {
    pub size: PageSize,
    pub offset: u64,
    pub resolve_total: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TotalRows {
    LowerBound(u64),
    Exact(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResultPagination {
    pub page_size: PageSize,
    pub offset: u64,
    pub visible_rows: usize,
    pub has_next: bool,
    pub total: TotalRows,
}
```

Keep calculations in checked `u64`. `from_page` receives the number of fetched rows including the optional probe and derives visible rows by clamping to `page_size`. Add helpers for first, previous, next, and exact-last requests; none may underflow or overflow.

**Step 5: Run focused tests and formatting**

Run: `cargo test model::pagination --lib`

Run: `cargo fmt --check`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/model/mod.rs src/model/pagination.rs
git commit -m "feat(model): add result pagination state"
```

### Task 2: Build Safe Page And Count SQL

**Files:**
- Modify: `src/sql/derived_result.rs`
- Modify: `src/sql/mod.rs`
- Test: `src/sql/derived_result.rs`

**Step 1: Replace fixed-cap expectations with failing page-query tests**

Add tests for all supported dialects requiring:

```text
SELECT * FROM (SELECT * FROM users) AS __lazydb_page LIMIT 501 OFFSET 0
SELECT COUNT(*) FROM (SELECT * FROM users) AS __lazydb_count
```

Require an existing user limit to remain inside the wrapper:

```text
SELECT * FROM (SELECT * FROM users LIMIT 20) AS __lazydb_page LIMIT 11 OFFSET 10
SELECT COUNT(*) FROM (SELECT * FROM users LIMIT 20) AS __lazydb_count
```

Add cases for a CTE, `UNION`, trailing semicolon, PostgreSQL `OFFSET`, and `FETCH`. Preserve existing rejection cases for multiple statements, mutations, `EXPLAIN`, and `FOR UPDATE`.

**Step 2: Add failing filtered-result tests**

Require `build_derived_query` to return a semantic base query without a fixed limit, then require the page builder to wrap that result. Verify submitted `WHERE` and `ORDER BY` appear once and that the count describes the filtered result.

**Step 3: Run focused tests**

Run: `cargo test sql::derived_result --lib`

Expected: FAIL because builders still hard-code or preserve `LIMIT 500`.

**Step 4: Implement one validated source abstraction**

Refactor parsing so one internal normalized read-query value supports:

```rust
pub struct PaginatedSql {
    pub page_sql: String,
    pub count_sql: String,
}

pub fn build_paginated_query(
    source: &str,
    dialect: SqlDialect,
    page: PageRequest,
) -> Result<PaginatedSql, DerivedQueryError>;
```

Use `page.size.lookahead_limit()` and checked offset conversion. Keep original user `LIMIT/OFFSET/FETCH` inside the source query. Do not construct count SQL for ineligible statements.

Change `build_derived_query` to build only the filtered semantic source or accept `PageRequest` and return `PaginatedSql`; choose the smaller API after updating its only App caller. Remove literal `LIMIT 500` and direct dependence on `RELATION_PREVIEW_LIMIT` from this module.

**Step 5: Run focused tests**

Run: `cargo test sql::derived_result --lib`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/sql/derived_result.rs src/sql/mod.rs
git commit -m "feat(sql): build paginated read queries"
```

### Task 3: Add Page Metadata To Relation Requests And Results

**Files:**
- Modify: `src/model/relation.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/db/query.rs`
- Modify: `src/model/tab.rs`
- Test: `tests/relation_runtime.rs`
- Test: `tests/relation_tabs.rs`

**Step 1: Write failing request-identity tests**

Extend relation request fixtures with `PageRequest`. Verify two otherwise equal requests with different offsets or page sizes are unequal and can independently identify cancellation/stale responses.

**Step 2: Write failing result-metadata tests**

Require `RelationPreview` to carry `ResultPagination`. Require new `RelationTab` and `ConsoleTab` values to default to 500 rows at offset zero. Require `DerivedResultState` to own pagination independently from its source result.

**Step 3: Run focused tests**

Run: `cargo test --test relation_runtime --test relation_tabs`

Expected: FAIL because requests and snapshots lack page data.

**Step 4: Extend typed boundaries**

- Add `page: PageRequest` to preview `RelationRequest` values. DDL requests can carry the default page request for minimal enum churn, but page data must not affect DDL execution.
- Add `pagination: ResultPagination` to `RelationPreview`.
- Add pagination state to `ConsoleTab` and `DerivedResultState` at the owner that corresponds to the displayed outcome.
- Keep `ResultSet` generic; do not put navigation state inside it.
- Replace `RELATION_PREVIEW_LIMIT` with a compatibility alias only if another non-pagination caller still needs it. Otherwise remove it after all usages migrate.

Update every test fixture explicitly rather than adding hidden compatibility constructors.

**Step 5: Run focused tests**

Run: `cargo test --test relation_runtime --test relation_tabs`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/model/relation.rs src/model/tab.rs src/db/mod.rs src/db/query.rs tests/relation_runtime.rs tests/relation_tabs.rs
git commit -m "feat(model): carry result page metadata"
```

### Task 4: Paginate Relation Preview Adapters

**Files:**
- Modify: `src/db/mod.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Test: adapter unit tests in the same files
- Test: `tests/catalog_contract.rs` if shared adapter contracts are extended

**Step 1: Extract and test relation SQL construction**

Before changing I/O, add pure adapter-local or shared helpers that build:

```text
SELECT * FROM <quoted relation> ... LIMIT 501 OFFSET 0
SELECT COUNT(*) FROM <quoted relation> ...
```

Verify each dialect's identifier quoting. Verify `WHERE` applies to both page and count, while outer `ORDER BY` applies only to the page. Test page sizes 10 and 1000 and a non-zero offset.

**Step 2: Run adapter-focused tests**

Run: `cargo test db::sqlite --lib`

Run: `cargo test db::postgres --lib`

Run: `cargo test db::mysql --lib`

Expected: FAIL until relation preview accepts page requests.

**Step 3: Update adapter signatures and ordinary page loading**

Change `DatabaseConnection::preview_relation` and all adapters to accept `PageRequest`. Fetch `page_size + 1`, decode at most `page_size` visible rows, and derive `ResultPagination` from the fetched count. Ensure `QueryStats.row_count` reports visible rows, not the probe.

Keep relation verification, catalog scope checks, SQL safety wrappers, and SQLx binding behavior unchanged.

**Step 4: Implement exact-last loading**

When `resolve_total` is true:

1. execute count SQL with `query_scalar::<_, i64>` or the adapter-safe unsigned conversion;
2. reject negative or overflowing counts;
3. compute the exact last offset through the shared model;
4. execute the page query using that offset;
5. return `TotalRows::Exact(total)`.

Acquire one connection for verification, count, and page query. On a non-zero exact total with an empty page, recount and retry once. Keep this correction in one small adapter-shared function if it can avoid duplicating logic without abstracting SQLx database types.

**Step 5: Run focused and contract tests**

Run: `cargo test db::sqlite db::postgres db::mysql --lib`

Run: `cargo test --test catalog_contract`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/db/mod.rs src/db/sqlite.rs src/db/postgres.rs src/db/mysql.rs tests/catalog_contract.rs
git commit -m "feat(db): paginate relation previews"
```

### Task 5: Wire Relation Page Actions Through App And Runtime

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Test: `tests/relation_runtime.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/workspace_tabs.rs`

**Step 1: Add failing reducer tests for page actions**

Add actions for first, previous, next, last, and selecting a page size. Tests must verify:

- next uses the current offset plus page size;
- previous saturates at zero;
- first requests offset zero;
- last sets `resolve_total` rather than guessing an offset;
- size change returns to offset zero;
- accepted pages reset selected row and vertical row offset;
- manual column widths and horizontal offset remain valid;
- a new request cancels the pending relation request;
- stale page responses are ignored.

**Step 2: Add failing edit-guard tests**

For an active relation mutation transaction, executable draft, or uncommitted edit state, every page-changing action must emit no database command and set the sanitized status message:

```text
Commit or roll back relation changes before changing pages
```

Rollback must restore page navigation. Successful commit refresh must request the first page and invalidate exact total state.

**Step 3: Run focused tests**

Run: `cargo test --test relation_tabs --test relation_runtime --test workspace_tabs`

Expected: FAIL because pagination actions do not exist.

**Step 4: Implement App page intent handling**

Add one shared page-intent enum or five explicit actions, preferring the smaller reducer match. Build a new `RelationRequest` through the existing `load_active_relation` path, preserving relation identity, connection, scope, filters, generation, cancellation, and previous snapshot behavior.

Reset pagination to first on refresh and submitted/cleared relation filters. Preserve exact total only for a page-size change where all query-identity fields remain unchanged.

Close record view and rebuild page-local edit rows only after accepting a successful page.

**Step 5: Pass page requests through Runtime**

Update `Runtime::load_relation` to pass `task_request.page` into `preview_relation`. Keep relation task keys based on the complete request so cancellation and duplicate suppression remain correct.

**Step 6: Run focused tests**

Run: `cargo test --test relation_tabs --test relation_runtime --test workspace_tabs`

Expected: PASS.

**Step 7: Commit**

```bash
git add src/action.rs src/app.rs src/runtime.rs tests/relation_runtime.rs tests/relation_tabs.rs tests/workspace_tabs.rs
git commit -m "feat(app): navigate relation result pages"
```

### Task 6: Paginate Auto-Commit SQL Console Results

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/model/tab.rs`
- Test: `tests/sql_execution.rs`
- Test: App unit tests in `src/app.rs`

**Step 1: Replace fixed-limit execution tests**

Update `tests/sql_execution.rs` to require a plain SELECT dispatch like:

```text
SELECT * FROM (SELECT 1) AS __lazydb_page LIMIT 501 OFFSET 0
```

Add tests proving a user `LIMIT 20` is now wrapped for pagination rather than dispatched unchanged. Preserve the mutation test requiring raw unchanged SQL.

**Step 2: Add failing SQL page-navigation tests**

After an eligible successful SELECT, test first/previous/next/last and page-size actions. Verify the generated request remains bound to `LastExecution.draft`, connection, target, source generation, and current filter query. Verify a new base execution and filter submission reset to the first page.

Add one ineligible multi-statement test proving pagination actions produce no command.

**Step 3: Run focused tests**

Run: `cargo test --test sql_execution`

Run: `cargo test app::tests --lib`

Expected: FAIL against fixed-cap dispatch.

**Step 4: Introduce typed paginated query commands**

Avoid sending unrelated count SQL through a sequence of generic `RunQuery` actions. Add a typed command carrying page and optional count SQL, for example:

```rust
Command::RunPaginatedQuery {
    connection: ConnectionIdentity,
    target: ExecutionTarget,
    tab_id: Uuid,
    source_generation: u64,
    page_generation: u64,
    page: PageRequest,
    page_sql: String,
    count_sql: String,
    derived: bool,
}
```

The exact shape may use separate base/derived variants if that keeps existing stale checks clearer. Do not overload `QueryOutcome` with count result sets.

**Step 5: Execute and normalize ordinary pages**

Runtime executes page SQL, removes the probe from the final result set, and sends page metadata with the outcome. Keep all preceding result sets untouched only for non-pageable execution; pageable SQL is restricted to one read-only query and should have one result set.

For last-page intent, execute count SQL, compute the last offset, rebuild page SQL from typed source/page data or carry a safe page template. Do not edit SQL text by replacing numeric literals. Prefer carrying the normalized source and invoking the shared builder in App before dispatch or a typed runtime query specification.

**Step 6: Accept only current page responses**

Check tab ID, source generation, page/derived generation, connection, target, and transaction generation where relevant. Preserve previous result while loading. On success, update pagination, outcome, grid selection, and result view. On failure, preserve previous outcome and expose the error without replacing it with Output view solely for a page-navigation failure.

**Step 7: Run focused tests**

Run: `cargo test --test sql_execution`

Run: `cargo test app::tests --lib`

Expected: PASS.

**Step 8: Commit**

```bash
git add src/action.rs src/app.rs src/runtime.rs src/model/tab.rs tests/sql_execution.rs
git commit -m "feat(sql): paginate console query results"
```

### Task 7: Paginate Filtered And Manual-Transaction SQL Results

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/runtime/transaction.rs`
- Modify: `src/model/tab.rs`
- Test: `tests/sql_execution.rs`
- Test: transaction tests in `src/runtime/transaction.rs`
- Test: App unit tests in `src/app.rs`

**Step 1: Write failing filtered-result tests**

Require submitted SQL result filters to dispatch a paginated filtered query with 501-row lookahead. Verify next-page and last-page requests retain the submitted filters and original successful `LastExecution` source. Clearing filters must restore the base source at offset zero.

**Step 2: Write failing manual-transaction tests**

Require page and count SQL to execute through the existing pinned transaction worker. Verify transaction generation is present in request/response identity. Verify pagination is rejected while another manual query is running and after transaction state becomes Aborted or OutcomeUnknown.

**Step 3: Run focused tests**

Run: `cargo test --test sql_execution`

Run: `cargo test runtime::transaction --lib`

Expected: FAIL because derived and manual paths still execute one fixed query.

**Step 4: Reuse one SQL page specification**

Build the filtered semantic source once from `LastExecution.draft.sql` and `DataQueryOptions`, then pass it to the same page/count builder used by base SQL. Store enough typed source state in `DerivedResultState` to rebuild subsequent pages without reading mutable editor text.

Add a transaction-worker request capable of executing count then page on its owned connection. It must return one typed page response and preserve cancellation semantics. Do not run the count through the pool while the page runs in the transaction.

**Step 5: Preserve transaction output semantics**

Initial base execution remains an execution event in the output log. Page navigation and count requests are view operations and should not append duplicate successful SQL execution entries. Errors appear in the result query status without changing transaction state unless the database reports an abort condition through existing logic.

**Step 6: Run focused tests**

Run: `cargo test --test sql_execution`

Run: `cargo test runtime::transaction --lib`

Run: `cargo test app::tests --lib`

Expected: PASS.

**Step 7: Commit**

```bash
git add src/action.rs src/app.rs src/runtime.rs src/runtime/transaction.rs src/model/tab.rs tests/sql_execution.rs
git commit -m "feat(sql): paginate filtered transaction results"
```

### Task 8: Render The Shared Pagination Bar And Selector

**Files:**
- Create: `src/ui/pagination.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/relation.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/model/workspace.rs` only if the selector is represented as an overlay
- Modify: `src/app.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/keymap.rs`
- Test: mouse unit tests in `src/input/mouse.rs`

**Step 1: Add failing render tests**

Render and assert these representative strings:

```text
1-500
of 501+
501-1000
of 1,234
0-0
of 0
```

Add parity tests for relation DATA and SQL RESULT SET. Verify disabled first/previous controls on page one, disabled next/last controls on an exact last page, and exact grouped totals after a last-page response.

Add a narrow-width test that keeps range, total, previous, and next text within the panel without overwriting the table.

**Step 2: Add failing hit-region and selector tests**

Require hit targets for first, previous, page-size selector, next, and last. Clicking them must map to the corresponding App actions. Opening the selector must render 10, 100, 250, 500, and 1000, highlight the current value, support Up/Down, apply with Enter, and close with Esc.

**Step 3: Run focused UI/input tests**

Run: `cargo test --test ui_render pagination`

Run: `cargo test --test keymap pagination`

Run: `cargo test input::mouse --lib`

Expected: FAIL because no pagination UI exists.

**Step 4: Implement a stateless shared renderer**

`ui::pagination` receives the current `ResultPagination`, loading state, available width, and `UiState`. It renders styled spans and registers hit regions only for enabled controls. Keep formatting helpers pure and unit tested.

Reserve one footer row below the grid in both `ui::relation::render_data` and `ui::render_data`. Remove relation text `[500 row limit]`; retain SQL provenance or snapshot provenance only in remaining width. Do not place pagination state in `UiState` beyond hit regions and popup geometry.

**Step 5: Implement selector lifecycle**

Use the existing overlay architecture if it can represent a compact choice popup without affecting unrelated overlays. Otherwise add a small result-local popup state to App, not to the renderer. Ensure focus and mouse mapping classify pagination targets as `Focus::Results`.

Do not assign existing Vim row navigation keys to page navigation. Selector-local Up/Down/Enter/Esc is allowed because the selector captures input.

**Step 6: Reset page-local views after accepted navigation**

Close record view, set selected row and row offset to zero, clamp selected column, and preserve valid horizontal state. Verify relation edit sessions are rebuilt only from visible rows and the lookahead row never appears.

**Step 7: Run focused UI/input tests**

Run: `cargo test --test ui_render pagination`

Run: `cargo test --test keymap pagination`

Run: `cargo test input::mouse --lib`

Expected: PASS.

**Step 8: Commit**

```bash
git add src/ui/pagination.rs src/ui/mod.rs src/ui/relation.rs src/input/mouse.rs src/input/keymap.rs src/model/workspace.rs src/app.rs tests/ui_render.rs tests/keymap.rs
git commit -m "feat(ui): add result pagination controls"
```

### Task 9: Complete Lifecycle And Concurrency Regressions

**Files:**
- Modify: `tests/connection_switch.rs`
- Modify: `tests/relation_runtime.rs`
- Modify: `tests/relation_tabs.rs`
- Modify: `tests/sql_execution.rs`
- Modify: `tests/ui_render.rs`
- Modify: relevant App unit tests in `src/app.rs`

**Step 1: Add stale-response matrix tests**

Verify page/count responses are ignored after each of:

- a newer page request;
- page-size change;
- filter submission or clear;
- new SQL execution;
- relation refresh;
- target or connection switch;
- tab close;
- transaction generation change.

**Step 2: Add exact-total invalidation tests**

Verify exact totals survive previous/next and page-size changes for the same query identity, but are invalidated by every result-changing event. Verify an ordinary short page can establish exact total without a count.

**Step 3: Add failure-preservation tests**

Verify page-query failure and count-query failure retain the previous visible outcome. Last-page count failure must keep `LowerBound` and permit retry. Verify concurrent shrink correction retries only once and cannot loop.

**Step 4: Run lifecycle suites**

Run: `cargo test --test connection_switch --test relation_runtime --test relation_tabs --test sql_execution --test ui_render`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/app.rs tests/connection_switch.rs tests/relation_runtime.rs tests/relation_tabs.rs tests/sql_execution.rs tests/ui_render.rs
git commit -m "test: cover result pagination lifecycle"
```

### Task 10: Remove Fixed-Cap Behavior And Verify The Repository

**Files:**
- Modify: `src/db/query.rs`
- Modify: `src/sql/derived_result.rs`
- Modify: `src/ui/relation.rs`
- Modify: relevant help/user documentation discovered during implementation
- Verify: all files changed by Tasks 1-9

**Step 1: Search for obsolete fixed-cap behavior**

Run: `rg "RELATION_PREVIEW_LIMIT|LIMIT 500|500 row limit|row cap" src tests docs`

Expected: only intentional historical design references or compatibility names remain. Production query builders and footer labels must have no fixed 500-row behavior.

**Step 2: Verify page-size safety**

Inspect all SQL construction and request boundaries. Confirm the page size comes only from `PageSize`, offsets use checked arithmetic, count values are validated, and no user-controlled numeric string is interpolated as a limit or offset.

**Step 3: Run formatting**

Run: `cargo fmt --check`

Expected: PASS. If it fails, run `cargo fmt`, inspect only formatting changes, then rerun the check.

**Step 4: Run all tests**

Run: `cargo test --all-targets`

Expected: PASS.

**Step 5: Run Clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS.

**Step 6: Inspect the final diff**

Run: `git status --short`

Run: `git diff --stat`

Run: `git diff -- src/model src/sql src/db src/action.rs src/app.rs src/runtime.rs src/ui src/input tests docs/plans/2026-08-31-result-pagination-design.md docs/plans/2026-08-31-result-pagination-implementation.md`

Confirm:

- unrelated worktree changes were not reverted or staged;
- relation and SQL result behavior use the same page model;
- unsupported SQL execution remains unchanged;
- probes never enter visible outcomes;
- count queries occur only for last-page intent;
- relation edits block every page-changing action;
- stale responses cannot replace current pages;
- narrow result panels remain readable.

**Step 7: Commit final cleanup**

```bash
git add src tests docs/plans/2026-08-31-result-pagination-design.md docs/plans/2026-08-31-result-pagination-implementation.md
git commit -m "feat: add paginated database results"
```

Before running this broad `git add`, replace it with explicit paths if unrelated
changes exist anywhere under `src`, `tests`, or `docs`. Never stage files that
were not changed for this implementation.

## Acceptance Criteria

- A new relation or eligible SQL result defaults to 500 visible rows.
- Users can select 10, 100, 250, 500, or 1000 rows per page.
- First, previous, next, and last controls work for relation and eligible SQL results.
- An initial full 500-row page with more data displays `1-500 of 501+`.
- A short page automatically displays an exact total.
- Last-page navigation performs an on-demand count and displays the exact total.
- User `LIMIT/OFFSET/FETCH` remains part of SQL query semantics.
- New queries, filters, refreshes, targets, and connection changes return to page one and invalidate stale totals.
- Active relation edits cannot be lost through pagination.
- Unsupported SQL remains executable but non-pageable.
- Previous data remains visible during page loading and after recoverable page/count failures.
- SQLite, PostgreSQL, and MySQL produce safe dialect-appropriate page/count SQL.
- `cargo fmt --check`, `cargo test --all-targets`, and strict Clippy all pass.
