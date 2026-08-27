# SQL Scope Whitespace Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make every position inside a SQL statement, including internal spaces, line breaks, and comments, resolve to that statement while preserving no-scope behavior for inter-statement trivia.

**Architecture:** Keep `resolve_scope` as the single source of truth used by execution, formatting, and current-statement rendering. Internally represent each scanned statement with separate execution and activation ranges: the execution range preserves the SQL sent downstream, while the activation range starts at the first real SQL code byte so leading whitespace and standalone comments do not claim the following statement.

**Tech Stack:** Rust 2024, the existing dialect-aware SQL scanner, Ratatui editor snapshots, Cargo integration tests.

---

### Task 1: Lock Down Cursor-to-Statement Semantics

**Files:**
- Modify: `tests/sql_scope.rs:83-124`

**Step 1: Add a failing test for internal whitespace**

Add a test that checks every whitespace byte inside a simple statement, including the exact reported case:

```rust
#[test]
fn whitespace_inside_statement_resolves_to_current_scope() {
    let text = "Select * from sys_user";

    for cursor in text
        .char_indices()
        .filter_map(|(index, character)| character.is_whitespace().then_some(index))
    {
        assert_eq!(
            resolve_scope(text, cursor, None, SqlDialect::Generic).map(|scope| scope.sql),
            Some(text.to_owned()),
            "cursor at byte {cursor} should resolve the statement",
        );
    }
}
```

**Step 2: Add a failing test for multiline and comment positions**

Verify that line breaks, indentation, and comments after SQL has started remain part of the active statement:

```rust
#[test]
fn multiline_whitespace_and_internal_comments_resolve_to_current_scope() {
    let text = "select\n  /* explain projection */\n  *\nfrom sys_user;";

    for marker in ["\n", "  /*", "explain", "\n  *", "\nfrom"] {
        let cursor = text.find(marker).unwrap();
        assert_eq!(
            resolve_scope(text, cursor, None, SqlDialect::Generic).map(|scope| scope.sql),
            Some(text.to_owned()),
            "cursor at {marker:?} should resolve the statement",
        );
    }
}
```

This test intentionally includes a comment position. Once the first SQL token has appeared, all bytes through the executable range belong to that statement, even if the cursor is not currently on a token.

**Step 3: Add a boundary regression test**

Cover adjacent statements so the first statement cannot capture the first byte of the second statement:

```rust
#[test]
fn adjacent_statement_start_belongs_to_the_second_statement() {
    let text = "select 1;select 2;";
    let cursor = text.find("select 2").unwrap();

    assert_eq!(
        resolve_scope(text, cursor, None, SqlDialect::Generic).map(|scope| scope.sql),
        Some("select 2;".to_owned())
    );
}
```

Keep the existing `cursor_on_semicolon_selects_statement_but_gap_does_not` test unchanged. It is the safety regression proving that an independent comment before the next SQL statement still has no scope.

**Step 4: Run the new tests and verify the current defect**

Run:

```bash
cargo test --test sql_scope whitespace_inside_statement_resolves_to_current_scope -- --nocapture
cargo test --test sql_scope multiline_whitespace_and_internal_comments_resolve_to_current_scope -- --nocapture
cargo test --test sql_scope adjacent_statement_start_belongs_to_the_second_statement -- --nocapture
```

Expected before implementation:

- The internal whitespace test fails because `cursor_is_code` rejects ASCII whitespace.
- The multiline test fails on whitespace and comment positions.
- The adjacent statement test fails because `statement_at` uses `cursor <= range.end` and may return the first statement at its exclusive end.

### Task 2: Separate Execution and Activation Ranges

**Files:**
- Modify: `src/sql/scope.rs:67-75`
- Modify: `src/sql/scope.rs:115-192`
- Modify: `src/sql/scope.rs:194-238`
- Test: `tests/sql_scope.rs`

**Step 1: Add an internal statement span type**

Place this private type near the scope-resolution helpers:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StatementSpan {
    execution: TextRange,
    activation: TextRange,
}
```

Do not export this type. Public callers still receive `TextRange` through `ScopeSource`, so no API or downstream model change is needed.

**Step 2: Extract first-code detection from `has_code`**

Replace the boolean-only trivia loop with a helper that returns the first byte containing actual SQL code:

```rust
fn first_code_offset(text: &str, dialect: SqlDialect) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
        } else if bytes[index] == b'-'
            && bytes.get(index + 1) == Some(&b'-')
            && dash_comment_allowed(bytes, index, dialect)
        {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if bytes[index] == b'#'
            && matches!(dialect, SqlDialect::MySql | SqlDialect::Generic)
        {
            index += 1;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = skip_block_comment(text, index);
        } else {
            return Some(index);
        }
    }
    None
}

fn has_code(text: &str, dialect: SqlDialect) -> bool {
    first_code_offset(text, dialect).is_some()
}
```

This preserves all current dialect-specific comment behavior and avoids creating a third SQL lexer. Do not alter `statement_boundaries`, `dash_comment_allowed`, or `skip_block_comment` in this change.

**Step 3: Build execution and activation ranges from the same scan**

Add a private scanner and make the public scanner project only its execution ranges:

```rust
pub fn scan_statements(text: &str, dialect: SqlDialect) -> Vec<TextRange> {
    scan_statement_spans(text, dialect)
        .into_iter()
        .map(|statement| statement.execution)
        .collect()
}

fn scan_statement_spans(text: &str, dialect: SqlDialect) -> Vec<StatementSpan> {
    statement_boundaries(text, dialect)
        .into_iter()
        .filter_map(|(start, end)| {
            let execution = meaningful_range(text, TextRange::new(start, end), dialect)?;
            let sql = execution.get(text)?;
            let activation_start = execution.start + first_code_offset(sql, dialect)?;
            Some(StatementSpan {
                execution,
                activation: TextRange::new(activation_start, execution.end),
            })
        })
        .collect()
}
```

The resulting semantics are:

- `execution` retains the current statement text, including leading comments already included by `meaningful_range`.
- `activation` starts at the first non-trivia SQL byte.
- Internal whitespace and comments are inside `activation` after SQL begins.
- Leading whitespace and standalone comments before the first SQL token are outside `activation`.
- Trailing inter-statement whitespace remains outside both ranges because `meaningful_range` already trims it.

**Step 4: Resolve the cursor against the half-open activation range**

Replace `statement_at` and delete `cursor_is_code` entirely:

```rust
fn statement_at(text: &str, cursor: usize, dialect: SqlDialect) -> Option<TextRange> {
    let cursor = cursor.min(text.len());
    scan_statement_spans(text, dialect)
        .into_iter()
        .find(|statement| {
            statement.activation.start <= cursor && cursor < statement.activation.end
        })
        .map(|statement| statement.execution)
}
```

Use the standard half-open interval `[start, end)`. Do not add a general `cursor <= end` fallback: it reintroduces ambiguity at adjacent statement boundaries. Do not add special EOF behavior unless an explicit failing editor test demonstrates that the editor can place its cursor one byte past an unterminated final statement.

**Step 5: Run the focused SQL scope test file**

Run:

```bash
cargo test --test sql_scope
```

Expected: all scope tests pass, including the existing Visual selection, UTF-8 byte range, semicolon, gap comment, dialect, and unterminated construct cases.

### Task 3: Verify Execution from Internal Whitespace

**Files:**
- Modify: `tests/sql_execution.rs:27-34`

**Step 1: Add an execution regression test**

Move the editor cursor from the initial `S` to the space after `SELECT`, execute the current scope, and assert that only the containing statement is dispatched:

```rust
#[test]
fn current_run_executes_statement_when_cursor_is_on_internal_space() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1; SELECT 2;".into()));
    for _ in 0.."SELECT".len() {
        app.update(Action::EditorKey(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )));
    }

    let commands = app.update(Action::RunActiveSql);

    assert!(matches!(
        commands.as_slice(),
        [Command::RunQuery { sql, .. }] if sql == "SELECT 1;"
    ));
}
```

This exercises the same path triggered by the editor's leader binding (`space+r`) after that binding emits `Action::RunActiveSql`; it deliberately tests command output rather than connecting to a database.

**Step 2: Run the focused execution tests**

Run:

```bash
cargo test --test sql_execution current_run_executes_statement_when_cursor_is_on_internal_space -- --nocapture
cargo test --test sql_execution current_run_does_not_fall_back_to_the_whole_buffer -- --nocapture
```

Expected: both tests pass, and the selected SQL is exactly `SELECT 1;` rather than the full buffer.

### Task 4: Verify Underline Projection from Internal Whitespace

**Files:**
- Modify: `tests/ui_render.rs:85-107`

**Step 1: Add a UI snapshot regression test**

Add a sibling test that moves the cursor onto the space after `SELECT` and confirms only the first statement is marked as current:

```rust
#[test]
fn sql_editor_underlines_statement_when_cursor_is_on_internal_space() {
    let mut app = fixture();
    app.update(Action::ReplaceEditor("SELECT 1;\nSELECT 2;".into()));
    for _ in 0.."SELECT".len() {
        app.update(Action::EditorKey(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )));
    }

    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 120,
            height: 10,
        })
        .unwrap();

    assert!(
        snapshot.lines[0]
            .spans
            .iter()
            .any(|span| span.current_statement)
    );
    assert!(
        snapshot.lines[1]
            .spans
            .iter()
            .all(|span| !span.current_statement)
    );
}
```

No production changes are expected in `src/app.rs`, `src/editor/mod.rs`, or `src/ui/mod.rs`. This test proves that fixing the shared scope resolver automatically reaches the existing `Modifier::UNDERLINED` path.

**Step 2: Run the focused UI tests**

Run:

```bash
cargo test --test ui_render sql_editor_underlines -- --nocapture
```

Expected: both the existing initial-cursor test and the new internal-space test pass.

### Task 5: Run Full Regression Verification

**Files:**
- Verify: `src/sql/scope.rs`
- Verify: `tests/sql_scope.rs`
- Verify: `tests/sql_execution.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Format the changed Rust files**

Run:

```bash
cargo fmt --check
```

If this reports formatting differences, run `cargo fmt`, inspect the resulting diff, and rerun `cargo fmt --check`.

Expected: formatting check exits successfully.

**Step 2: Run static analysis**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: no warnings or errors.

**Step 3: Run the complete Rust test suite**

Run:

```bash
cargo test --all-targets --all-features
```

Expected: all tests pass. PostgreSQL and MySQL adapter tests may remain environment-gated as documented in `CONTRIBUTING.md`; no external database is required for the new tests.

**Step 4: Inspect the final diff**

Run:

```bash
git diff --check
git diff -- src/sql/scope.rs tests/sql_scope.rs tests/sql_execution.rs tests/ui_render.rs
```

Expected:

- No whitespace errors.
- Production changes are limited to the private scope scanner in `src/sql/scope.rs`.
- No keymap, command dispatch, editor rendering, or public SQL API changes are present.
- Tests cover pure scope behavior, execution dispatch, and underline projection.

**Step 5: Perform a manual terminal smoke test when an interactive database session is available**

Verify:

- Enter `Select * from sys_user` and move the cursor onto each internal space; the complete statement remains underlined.
- Press `space+r` from an internal space; the statement runs instead of showing `No SQL scope at cursor`.
- With `select 1;\n\n-- gap\n\nselect 2;`, place the cursor on `-- gap`; no statement is underlined and execution still reports no scope.
- With `select 1;select 2;`, place the cursor on the second `s`; only `select 2;` is underlined and executed.

No commit should be created unless explicitly requested by the user.
