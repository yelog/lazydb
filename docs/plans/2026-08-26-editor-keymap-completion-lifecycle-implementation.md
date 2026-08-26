# Editor Keymap and Completion Lifecycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restore Normal-mode global key ownership, make Insert Escape exit the mode while dismissing completion, and prevent automatic completion from reopening after accepted closed identifiers.

**Architecture:** Keep existing Actions and App reducer ordering. Reorder mode-aware key ownership in `Keymap::map`, add one automatic-completion eligibility predicate in the SQL completion module, and use that predicate before immediate or delayed completion scheduling.

**Tech Stack:** Rust 1.94, Crossterm 0.29, modalkit 0.0.25, existing App/keymap/completion tests.

---

## Preconditions

- Work only in `/Users/yelog/workspace/tui/lazydb-sqleditor` on `task/sqleditor`.
- Read `docs/plans/2026-08-26-editor-keymap-completion-lifecycle-design.md`.
- Do not stage or modify the unrelated untracked `docs/plans/2026-08-25-database-explorer-implementation.md`.
- Keep `App::update` as the mutation boundary.
- Do not add a compound Escape action or a `just_accepted_completion` flag.

### Task 1: Restore Normal-Mode Global Keys

**Files:**
- Modify: `tests/keymap.rs`
- Modify: `src/input/keymap.rs:121-202`

**Step 1: Add failing keymap tests**

Add one table-driven test for Normal mode with and without a popup:

```rust
#[test]
fn normal_mode_global_keys_win_over_editor_and_completion() {
    for popup in [false, true] {
        let mut keymap = Keymap::default();
        let mut app = App::new(Vec::new());
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        if popup {
            app.active_console_mut().completion = Some(CompletionPopup::default());
        }

        assert_eq!(keymap.map(key(KeyCode::Char('?')), &app), Some(Action::ShowHelp));
        assert_eq!(keymap.map(key(KeyCode::Tab), &app), Some(Action::FocusNext));
        assert_eq!(keymap.map(key(KeyCode::BackTab), &app), Some(Action::FocusPrevious));
    }
}
```

**Step 2: Run and observe failure**

Run:

```bash
cargo test --test keymap normal_mode_global_keys_win_over_editor_and_completion -- --nocapture
```

Expected: FAIL because Editor focus returns `EditorKey` before the global `?`,
Tab, and BackTab branches.

**Step 3: Reorder mode-aware ownership**

After modal-overlay routing and before completion routing, add:

```rust
if app.focus == Focus::Editor && app.active_editor_mode() == EditorMode::Normal {
    match event.code {
        KeyCode::Char('?') => return Some(Action::ShowHelp),
        KeyCode::Tab => return Some(Action::FocusNext),
        KeyCode::BackTab => return Some(Action::FocusPrevious),
        _ => {}
    }
}
```

Keep Insert Tab and BackTab routed to modalkit unless separately specified.

**Step 4: Run keymap regressions**

Run:

```bash
cargo test --test keymap
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/input/keymap.rs tests/keymap.rs
git commit -m "fix(keymap): preserve normal mode global keys"
```

### Task 2: Make Insert Escape Exit Mode With Completion Visible

**Files:**
- Modify: `tests/keymap.rs`
- Modify: `tests/app_flow.rs`
- Modify: `src/input/keymap.rs:121-136`

**Step 1: Add failing mapping test**

```rust
#[test]
fn insert_escape_bypasses_completion_dismiss() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.active_console_mut().completion = Some(CompletionPopup::default());
    let escape = key(KeyCode::Esc);

    assert_eq!(keymap.map(escape, &app), Some(Action::EditorKey(escape)));
}
```

**Step 2: Add failing App pipeline test**

Create a non-empty popup, map Escape through Keymap, apply the resulting Action,
and assert:

```rust
assert!(app.active_console().completion.is_none());
assert_eq!(app.active_editor_mode(), EditorMode::Normal);
assert_eq!(app.active_editor_text().unwrap(), original);
```

**Step 3: Run and observe failure**

Run:

```bash
cargo test --test keymap insert_escape_bypasses_completion_dismiss -- --nocapture
cargo test --test app_flow escape_with_completion -- --nocapture
```

Expected: keymap returns `CompletionDismiss`; App remains Insert.

**Step 4: Change completion ownership**

Before the generic completion branch, route Insert Escape:

```rust
if app.focus == Focus::Editor
    && app.active_editor_mode() == EditorMode::Insert
    && event.code == KeyCode::Esc
{
    return Some(Action::EditorKey(event));
}
```

Remove Escape from the completion-owned match. Keep Ctrl-N, Ctrl-P, and Enter.

**Step 5: Run focused and editor tests**

Run:

```bash
cargo test --test keymap --test app_flow
cargo test editor::tests::session_starts_insert_and_transitions_with_escape_and_i --lib
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/input/keymap.rs tests/keymap.rs tests/app_flow.rs
git commit -m "fix(completion): let escape exit insert mode"
```

### Task 3: Add Automatic Completion Eligibility

**Files:**
- Modify: `src/sql/completion.rs:87-197,255-284`
- Modify: `src/sql/mod.rs`
- Modify: `tests/sql_completion.rs`

**Step 1: Add failing boundary tests**

Expose a domain predicate and test:

```rust
assert!(should_offer_completion("tools.sys", 9));
assert!(should_offer_completion("tools.", 6));
assert!(!should_offer_completion("tools.\"sys_config\"", 18));
assert!(!should_offer_completion("tools.`sys_config`", 18));
assert!(!should_offer_completion("SELECT * FROM tools;", 20));
assert!(!should_offer_completion("tools.sys ", 10));
assert!(!should_offer_completion("tools.sys)", 10));
```

Use actual string lengths in the final test rather than brittle constants.

**Step 2: Run and observe failure**

Run:

```bash
cargo test --test sql_completion completion_boundaries -- --nocapture
```

Expected: FAIL because the predicate does not exist.

**Step 3: Implement two byte predicates**

Keep replacement scanning behavior:

```rust
fn is_identifier_scan_byte(byte: u8, dialect: SqlDialect) -> bool
```

Add automatic continuation behavior:

```rust
fn is_completion_continuation(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}
```

Implement eligibility:

```rust
pub fn should_offer_completion(text: &str, cursor: usize) -> bool {
    let cursor = cursor.min(text.len());
    let Some(previous) = cursor.checked_sub(1).and_then(|i| text.as_bytes().get(i)) else {
        return false;
    };
    *previous == b'.' || is_completion_continuation(*previous)
}
```

Do not remove quote handling from replacement scanning.

**Step 4: Run completion tests**

Run:

```bash
cargo test --test sql_completion
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/sql/completion.rs src/sql/mod.rs tests/sql_completion.rs
git commit -m "fix(completion): define automatic trigger boundaries"
```

### Task 4: Apply Eligibility to Scheduling and Acceptance

**Files:**
- Modify: `src/app.rs:599-613,2119-2133,2270-2310`
- Modify: `tests/app_flow.rs`
- Modify: `tests/sql_completion.rs`

**Step 1: Add failing quoted-acceptance integration test**

Create catalog fixtures containing schema `tools` and relation `sys_config`, then
exercise:

```text
SELECT * FROM tools.sys
CompletionExplicit
select sys_config
CompletionAccept
```

Assert:

```rust
assert_eq!(text, "SELECT * FROM tools.\"sys_config\"");
assert_eq!(cursor, text.chars().count());
assert!(app.active_console().completion.is_none());
```

Also assert typing `.` after a qualified identifier can schedule completion again.

**Step 2: Run and observe failure**

Run:

```bash
cargo test --test app_flow quoted_completion_acceptance -- --nocapture
```

Expected: popup reopens because the replacement `Changed` effect schedules a new
completion and quote bytes remain part of identifier scanning.

**Step 3: Gate automatic completion in `apply_editor_effects`**

For `EditorEffect::Changed`, obtain text and cursor byte from the current render
snapshot. Only call `complete_now` or enqueue `ScheduleCompletion` when
`should_offer_completion` is true. Otherwise ensure completion remains `None`.

Preserve explicit `Action::CompletionExplicit`, which calls `complete_now`
without the automatic predicate.

**Step 4: Keep acceptance revision and history unchanged**

Do not suppress `Changed`, skip revision increments, or special-case accepted
candidates. The normal predicate handles the closed quote.

**Step 5: Run focused regressions**

Run:

```bash
cargo test --test app_flow --test sql_completion --test keymap
```

Expected: PASS.

**Step 6: Run complete gates**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

Expected: PASS.

**Step 7: Commit**

```bash
git add src/app.rs tests/app_flow.rs tests/sql_completion.rs
git commit -m "fix(completion): close popup after accepted identifier"
```

## Acceptance Checklist

- Normal `?` opens Help with or without a popup.
- Normal Tab and Shift-Tab change panel focus with or without a popup.
- Insert Escape with a popup enters Normal and closes the popup.
- Popup Ctrl-N, Ctrl-P, and Enter still work.
- Closed quoted identifiers do not auto-trigger completion.
- Qualified prefixes and qualifier dots still offer completion.
- Accepted completion remains undoable and leaves the cursor on the right.
- Complete formatting, Clippy, test, and diff gates pass.
