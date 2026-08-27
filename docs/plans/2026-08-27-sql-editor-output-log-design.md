# SQL Editor Output Log Design

**Goal:** Make SQL execution focus the appropriate result tab and append actionable execution details to the OUTPUT log.

**Scope:** Query executions focus `DATA`; non-query executions and failures focus `OUTPUT`. Each completed execution appends a timestamped SQL/target line and a statistics line while preserving existing output entries.

## Design

- Reuse `ExecutionDraft` for the exact executed SQL and execution target.
- Reuse `QueryStats` for execution, fetching, and total durations.
- Keep formatting in the App layer so database drivers remain concerned only with execution and measurement.
- Use the SQL risk classification already captured by `ExecutionDraft` to distinguish read-only queries from non-query statements.
- Keep the existing `OutputEntry` model and store the two display lines as two consecutive entries, allowing the current output renderer to keep its scrolling behavior.
- Sanitize SQL, target, profile names, and error text before rendering terminal output.
- Keep derived-query executions out of the main execution log because they are result filtering operations, not user-submitted SQL executions.

## Behavior

- Successful read-only query: append execution details and select `DATA`.
- Successful non-query: append execution details and select `OUTPUT`.
- Failed execution: select `OUTPUT` and preserve the error entry; when execution metadata is available, include the submitted SQL and target context.
- Manual transaction statements use the same output formatting and tab-selection rules.
- Existing informational, cancellation, and connection messages remain append-only.

## Example

```text
[2026-08-27 18:26:45:431] kms.public> select * from tools.sys_user
[2026-08-27 18:26:45:849] 137 rows retrieved starting from 1 in 418 ms (execution: 21 ms, fetching: 397 ms)
```

## Verification

- Query completion selects `DATA` and records SQL, target, and all timing fields.
- Non-query completion selects `OUTPUT` and records affected-row information and all timing fields.
- Multiple executions append rather than replace output history.
- Manual execution follows the same rules.
- Failure remains focused on `OUTPUT`.
