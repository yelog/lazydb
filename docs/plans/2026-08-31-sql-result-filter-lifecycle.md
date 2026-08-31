# SQL Result Filter Lifecycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the stale SQL result-filter placeholder with an explicit waiting state and guarantee that derived WHERE/ORDER BY queries can only replay the latest successful eligible SQL execution.

**Architecture:** Keep the existing `DataQueryState` and server-side derived-query implementation, but add an `AwaitingResult` capability for normal pre-result and running states. Centralize SQL filter invalidation at base-query dispatch, recompute capability after terminal execution events, and enforce the same invariant again at `submit_sql_query()` so UI routing is not the security boundary.

**Tech Stack:** Rust 2024, Ratatui, SQLParser, SQLx, Cargo test, Clippy

---

### Task 1: Model the Normal Waiting State

**Files:**
- Modify: `src/model/data_query.rs:39-50`
- Modify: `src/ui/query_bar.rs:25-78`
- Modify: `src/ui/mod.rs:1832-1849`
- Modify: `src/input/keymap.rs:1339-1359`
- Test: `tests/ui_render.rs:1584-1612`

**Step 1: Write the failing initial-state rendering test**

Add a fixture that does not synthesize `Action::QueryFinished`, then assert that the SQL DATA view:

```rust
#[test]
fn sql_data_before_first_execution_has_a_quiet_disabled_query_bar() {
    let profile = import_connection_url("sqlite::memory:", Some("orbital-lab"))
        .unwrap()
        .profile;
    let app = App::new(vec![profile]);

    let (output, state) = render_with_state(&app, 120, 36);

    assert!(output.contains("WHERE"), "{output}");
    assert!(output.contains("ORDER BY"), "{output}");
    assert!(output.contains("Run a query to populate the data viewport"), "{output}");
    assert!(!output.contains("SQL result filtering is not implemented yet"), "{output}");
    assert!(!output.contains("Run a read-only query first"), "{output}");
    assert!(!state.hit_regions.iter().any(|region| {
        matches!(region.target, HitTarget::DataQueryInput(_))
    }));
}
```

Keep `sql_query_bar_is_inert_until_derived_execution_exists`; update its name to `sql_query_bar_is_inert_while_awaiting_a_result` if that makes the intended state clearer.

**Step 2: Run the test to verify it fails**

Run:

```bash
cargo test --test ui_render sql_data_before_first_execution_has_a_quiet_disabled_query_bar --all-features
```

Expected: FAIL because the output contains `SQL result filtering is not implemented yet`.

**Step 3: Add `AwaitingResult` and make it the default**

Change the capability model to:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataQueryCapability {
    Relation,
    Sql,
    AwaitingResult,
    Unavailable(String),
}

impl Default for DataQueryCapability {
    fn default() -> Self {
        Self::AwaitingResult
    }
}
```

Do not store a display string in `AwaitingResult`; it represents a normal lifecycle state, not an error.

**Step 4: Update exhaustive capability matches**

Preserve query-bar enablement as exactly:

```rust
matches!(
    query.capability,
    DataQueryCapability::Relation | DataQueryCapability::Sql
)
```

Update `src/ui/query_bar.rs` so `AwaitingResult` has no message:

```rust
let message = query.error.as_ref().or(match &query.capability {
    DataQueryCapability::Unavailable(reason) => Some(reason),
    DataQueryCapability::Relation
    | DataQueryCapability::Sql
    | DataQueryCapability::AwaitingResult => None,
});
```

Update `src/ui/mod.rs` so query-bar height depends on whether a message can actually be rendered, not whether the capability is enabled:

```rust
let query_height = if tab.query.error.is_some()
    || matches!(
        tab.query.capability,
        crate::model::data_query::DataQueryCapability::Unavailable(_)
    )
{
    3
} else {
    2
};
```

This gives `AwaitingResult` two rows and leaves the existing result-set empty message as the single call to action. `src/input/keymap.rs` should continue returning `None` for every capability other than `Relation | Sql`; only make the exhaustive match compile.

**Step 5: Run focused UI tests**

Run:

```bash
cargo test --test ui_render sql_data_before_first_execution_has_a_quiet_disabled_query_bar --all-features
cargo test --test ui_render sql_query_bar_is_inert_while_awaiting_a_result --all-features
```

Expected: PASS. If the existing inert test keeps its old name, run that exact name instead.

**Step 6: Inspect the focused diff**

Run:

```bash
git diff -- src/model/data_query.rs src/ui/query_bar.rs src/ui/mod.rs src/input/keymap.rs tests/ui_render.rs
```

Expected: only the new lifecycle variant, exhaustive-match handling, layout condition, and tests are present. Do not commit unless the user explicitly asks.

---

### Task 2: Invalidate Filters When a New Base Query Is Dispatched

**Files:**
- Modify: `src/app.rs:5794-5853`
- Test: `src/app.rs:8672-8755`

**Step 1: Write the failing dispatch lifecycle test**

In `src/app.rs`'s test module, import:

```rust
use crate::model::data_query::{
    DataQueryCapability, DataQueryInput, DataQueryOptions,
};
use crate::model::tab::DerivedResultState;
```

Use `connected_query_app()` to finish a successful SELECT, populate filter state, and dispatch a second base query:

```rust
#[test]
fn dispatching_a_new_base_query_invalidates_sql_filter_state() {
    let (mut app, tab_id, generation) = connected_query_app("SELECT id FROM users");
    let connection = app.connection.active_identity().unwrap();
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection,
        outcome: empty_outcome(),
    });
    {
        let tab = app.active_console_mut();
        tab.query.where_input.set("id > 10");
        tab.query.order_by_input.set("id DESC");
        tab.query.submitted = DataQueryOptions {
            where_clause: Some("id > 10".into()),
            order_by_clause: Some("id DESC".into()),
        };
        tab.query.focus = Some(DataQueryInput::Where);
        tab.query.error = Some("old error".into());
        tab.derived = Some(DerivedResultState {
            source: tab.last_execution.clone().unwrap(),
            query: tab.query.submitted.clone(),
            generation: 1,
            outcome: Some(empty_outcome()),
            error: None,
            running: false,
        });
    }

    app.update(Action::ReplaceEditor("SELECT name FROM users".into()));
    let commands = app.update(Action::RunActiveSql);

    assert!(matches!(commands.as_slice(), [Command::RunQuery { .. }]));
    let tab = app.active_console();
    assert_eq!(tab.query.capability, DataQueryCapability::AwaitingResult);
    assert_eq!(tab.query.where_input.value(), "");
    assert_eq!(tab.query.order_by_input.value(), "");
    assert_eq!(tab.query.submitted, DataQueryOptions::default());
    assert_eq!(tab.query.focus, None);
    assert_eq!(tab.query.error, None);
    assert_eq!(tab.query.completion, None);
    assert_eq!(tab.derived, None);
}
```

Use SQL that does not require confirmation under the test policy, or follow the existing helper's confirmation path.

**Step 2: Run the test to verify it fails**

Run:

```bash
cargo test app::tests::dispatching_a_new_base_query_invalidates_sql_filter_state --all-features
```

Expected: FAIL because the old capability, filter inputs, and derived result remain.

**Step 3: Add one SQL-filter reset helper**

Add a private helper near the other console state helpers in `src/app.rs`:

```rust
fn reset_sql_filter_for_base_execution(tab: &mut ConsoleTab) {
    tab.query.where_input.set("");
    tab.query.order_by_input.set("");
    tab.query.submitted = DataQueryOptions::default();
    tab.query.focus = None;
    tab.query.error = None;
    tab.query.capability = DataQueryCapability::AwaitingResult;
    tab.query.completion = None;
    tab.derived = None;
}
```

Keep this helper SQL-console-specific. Do not alter relation filtering state.

**Step 4: Call the helper only after dispatch is committed**

Call `reset_sql_filter_for_base_execution(tab)` in both successful branches of `dispatch_draft()`:

```rust
tab.generation += 1;
reset_sql_filter_for_base_execution(tab);
tab.query_status = QueryStatus::Running;
tab.last_execution = Some(...);
```

Apply it to automatic execution and the manual transaction branch immediately before storing the new `LastExecution`. Do not clear filter state during validation, confirmation display, or any path that returns without a database command.

Preserve `tab.outcome`: running a new query may continue showing the previous base result beneath the loading indicator, but it must not show or submit the previous derived result.

**Step 5: Run dispatch and existing execution tests**

Run:

```bash
cargo test app::tests::dispatching_a_new_base_query_invalidates_sql_filter_state --all-features
cargo test --test sql_execution --all-features
```

Expected: PASS.

**Step 6: Inspect the focused diff**

Run:

```bash
git diff -- src/app.rs
```

Expected: one reset helper, calls from the two committed dispatch branches, and the focused unit test. Do not commit unless explicitly requested.

---

### Task 3: Make Terminal Execution States Authoritative

**Files:**
- Modify: `src/app.rs:2550-2606`
- Modify: `src/app.rs:2760-2804`
- Modify: `src/app.rs:3401-3428`
- Modify: `src/app.rs:8240-8302`
- Test: `src/app.rs:8672-8755`

**Step 1: Extend the success test with capability assertions**

In `query_completion_focuses_data_and_records_execution_details`, add:

```rust
assert_eq!(
    tab.query.capability,
    DataQueryCapability::Sql
);
assert_eq!(
    tab.last_execution.as_ref().map(|last| &last.result),
    Some(&ExecutionResult::Succeeded)
);
```

Import `ExecutionResult` alongside the existing tab model imports.

**Step 2: Write failing failure and cancellation tests**

Add a failure test that starts from a dispatched read-only SELECT and then sends `Action::QueryFailed`:

```rust
#[test]
fn failed_base_query_keeps_sql_filtering_unavailable() {
    let (mut app, tab_id, generation) = connected_query_app("SELECT * FROM missing_table");
    let connection = app.connection.active_identity().unwrap();

    app.update(Action::QueryFailed {
        tab_id,
        generation,
        connection,
        message: "missing table".into(),
    });

    let tab = app.active_console();
    assert!(matches!(
        tab.query.capability,
        DataQueryCapability::Unavailable(ref reason)
            if reason == "Run a successful read-only SELECT query to enable filtering"
    ));
    assert_eq!(
        tab.last_execution.as_ref().map(|last| &last.result),
        Some(&ExecutionResult::Failed)
    );
    assert!(app.update(Action::SubmitDataQuery).is_empty());
}
```

Add a cancellation test using the existing `Action::CancelActiveQuery` action:

```rust
#[test]
fn cancelled_base_query_keeps_sql_filtering_unavailable() {
    let (mut app, _, _) = connected_query_app("SELECT * FROM users");

    let commands = app.update(Action::CancelActiveQuery);

    assert!(matches!(commands.as_slice(), [Command::CancelQuery { .. }]));
    let tab = app.active_console();
    assert!(matches!(
        tab.query.capability,
        DataQueryCapability::Unavailable(ref reason)
            if reason == "Run a successful read-only SELECT query to enable filtering"
    ));
    assert_eq!(
        tab.last_execution.as_ref().map(|last| &last.result),
        Some(&ExecutionResult::Cancelled)
    );
}
```

**Step 3: Run the tests to verify the required behavior is missing**

Run:

```bash
cargo test app::tests::failed_base_query_keeps_sql_filtering_unavailable --all-features
cargo test app::tests::cancelled_base_query_keeps_sql_filtering_unavailable --all-features
```

Expected: at least one FAIL because failure/cancellation paths do not explicitly set the disabled capability.

**Step 4: Add one terminal unavailable reason helper**

Avoid duplicating the user-facing string:

```rust
fn unavailable_sql_filter_after_unsuccessful_execution() -> DataQueryCapability {
    DataQueryCapability::Unavailable(
        "Run a successful read-only SELECT query to enable filtering".into(),
    )
}
```

Use this helper when a dispatched base query fails or is cancelled. Do not use it for an unsupported but successful statement; that state retains the more specific reason `Filtering requires one read-only SELECT query`.

**Step 5: Make success ordering explicit**

In the query-finish handler, mark the matching `LastExecution` as `Succeeded` before computing capability. Then derive capability only from a successful matching execution:

```rust
if let Some(last) = tab.last_execution.as_mut()
    && last.draft.query_generation + 1 == generation
{
    last.result = ExecutionResult::Succeeded;
}

tab.query.capability = match tab.last_execution.as_ref() {
    Some(last)
        if last.result == ExecutionResult::Succeeded
            && sql::derived_query_capable(&last.draft.sql, last.draft.dialect) =>
    {
        DataQueryCapability::Sql
    }
    Some(last) if last.result == ExecutionResult::Succeeded => {
        DataQueryCapability::Unavailable(
            "Filtering requires one read-only SELECT query".into(),
        )
    }
    _ => unavailable_sql_filter_after_unsuccessful_execution(),
};
```

Remove the later duplicate assignment to `last.result`.

**Step 6: Update failure and cancellation paths**

After setting `ExecutionResult::Failed` in `Action::QueryFailed`, set:

```rust
tab.query.capability = unavailable_sql_filter_after_unsuccessful_execution();
```

In normal cancellation and confirmed manual cancellation, apply the same capability after invalidating/marking the execution. Keep `query.error` reserved for fragment or derived-query errors; the capability reason is enough for base execution failure/cancellation.

**Step 7: Run lifecycle tests**

Run:

```bash
cargo test app::tests::query_completion_focuses_data_and_records_execution_details --all-features
cargo test app::tests::failed_base_query_keeps_sql_filtering_unavailable --all-features
cargo test app::tests::cancelled_base_query_keeps_sql_filtering_unavailable --all-features
```

Expected: PASS.

**Step 8: Run transaction-focused tests**

Run:

```bash
cargo test --test sqlite_transactions --all-features
cargo test --test transaction_sql --all-features
```

Expected: PASS. This confirms manual dispatch/cancellation changes do not alter transaction semantics.

---

### Task 4: Defend Derived Submission at the Controller Boundary

**Files:**
- Modify: `src/app.rs:6972-7037`
- Test: `src/app.rs:8606-9626`

**Step 1: Write the failing direct-action test**

Construct a console containing filter text and a non-successful `LastExecution`, then invoke the action directly rather than through the keymap:

```rust
#[test]
fn derived_submission_requires_a_successful_sql_capability() {
    let (mut app, _, _) = connected_query_app("SELECT id FROM users");
    {
        let tab = app.active_console_mut();
        tab.query.capability = DataQueryCapability::Sql;
        tab.query.where_input.set("id > 10");
        assert_eq!(
            tab.last_execution.as_ref().map(|last| &last.result),
            Some(&ExecutionResult::Dispatched)
        );
    }

    assert!(app.update(Action::SubmitDataQuery).is_empty());
    assert_eq!(app.active_console().derived, None);
}
```

This deliberately creates a mismatched state to prove the controller does not trust UI capability alone.

Add a second assertion or test for `ExecutionResult::Failed` if it can be expressed without duplicating most setup.

**Step 2: Run the test to verify it fails**

Run:

```bash
cargo test app::tests::derived_submission_requires_a_successful_sql_capability --all-features
```

Expected: FAIL because current `submit_sql_query()` only checks that `last_execution` exists.

**Step 3: Add fail-closed guards to `submit_sql_query()`**

At the top of the method, replace the current existence-only lookup with:

```rust
let tab = self.active_console();
if !matches!(tab.query.capability, DataQueryCapability::Sql) {
    return Vec::new();
}
let Some(last) = tab
    .last_execution
    .as_ref()
    .filter(|last| last.result == ExecutionResult::Succeeded)
    .cloned()
else {
    return Vec::new();
};
```

Keep `build_derived_query()` unchanged so it remains the final SQL parsing, read-only, and fragment-validation boundary. Do not add compatibility fallback behavior.

**Step 4: Add a positive derived-submission test**

Finish an eligible query, focus the WHERE field, insert a filter, and submit:

```rust
#[test]
fn successful_read_only_result_submits_a_derived_query() {
    let (mut app, tab_id, generation) = connected_query_app("SELECT id FROM users");
    let connection = app.connection.active_identity().unwrap();
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection,
        outcome: empty_outcome(),
    });
    app.update(Action::FocusDataQueryInput(DataQueryInput::Where));
    for character in "id > 10".chars() {
        app.update(Action::DataQueryInsert(character));
    }

    let commands = app.update(Action::SubmitDataQuery);

    assert!(matches!(
        commands.as_slice(),
        [Command::RunDerivedQuery { sql, .. }]
            if sql.contains("WHERE id > 10") && sql.ends_with("LIMIT 500")
    ));
}
```

**Step 5: Run positive and negative submission tests**

Run:

```bash
cargo test app::tests::derived_submission_requires_a_successful_sql_capability --all-features
cargo test app::tests::successful_read_only_result_submits_a_derived_query --all-features
```

Expected: PASS.

**Step 6: Run existing derived-query builder tests**

Run:

```bash
cargo test sql::derived_result --all-features
```

Expected: PASS. No SQL wrapper behavior should change.

---

### Task 5: Complete Regression Verification

**Files:**
- Verify: `src/model/data_query.rs`
- Verify: `src/app.rs`
- Verify: `src/ui/query_bar.rs`
- Verify: `src/ui/mod.rs`
- Verify: `src/input/keymap.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Search for stale and duplicated wording**

Run:

```bash
rg -n "SQL result filtering is not implemented yet|Run a read-only query first|Filtering requires one read-only SELECT query|Run a successful read-only SELECT query" src tests
```

Expected:

- No occurrence of `SQL result filtering is not implemented yet`.
- No initial-state use of `Run a read-only query first`.
- One stable unsupported-success reason.
- One stable unsuccessful-execution reason, preferably centralized in a helper.

**Step 2: Format the code**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
```

Expected: both commands succeed and the check produces no diff.

**Step 3: Run focused test targets**

Run:

```bash
cargo test --test ui_render --all-features
cargo test --test sql_execution --all-features
cargo test sql::derived_result --all-features
cargo test app::tests --all-features
```

Expected: PASS.

**Step 4: Run the complete test suite**

Run:

```bash
cargo test --all-features --all-targets
```

Expected: PASS.

**Step 5: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --all-features --all-targets -- -D warnings
```

Expected: PASS with no warnings.

**Step 6: Validate the final diff**

Run:

```bash
git diff --check
git status --short
git diff -- src/model/data_query.rs src/app.rs src/ui/query_bar.rs src/ui/mod.rs src/input/keymap.rs tests/ui_render.rs
```

Expected: no whitespace errors; only the intended lifecycle, rendering, controller guards, and tests are included. Preserve all unrelated worktree changes and do not commit unless explicitly requested.

**Step 7: Manual acceptance check**

Run LazyDB against a disposable database and verify:

1. A fresh SQL Editor shows disabled `WHERE`/`ORDER BY` and only `Run a query to populate the data viewport`; no warning is shown.
2. A successful single read-only SELECT enables both query fields.
3. Applying WHERE or ORDER BY executes a derived query and displays its result.
4. Starting another base query immediately disables and clears the old filters while retaining the previous base result only as a loading fallback.
5. A failed or cancelled base query leaves filtering disabled with the successful-read-only-SELECT guidance.
6. A successful UPDATE, multi-statement query, EXPLAIN, or lock-bearing query leaves filtering disabled with the one-read-only-SELECT reason.
7. Clearing both clauses after a derived result restores the base result without database I/O.
