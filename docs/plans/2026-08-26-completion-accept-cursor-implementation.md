# Completion Acceptance and Cursor Authority Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep completion closed after acceptance and make the real modalkit leader match the displayed post-replacement cursor.

**Architecture:** Preserve normal revision and undo effects from programmatic replacement. Add a local effect-drain policy for completion acceptance and synchronize `EditBuffer` leader state after every range replacement.

**Tech Stack:** Rust 1.94, modalkit 0.0.25, existing App/editor/completion tests.

---

## Preconditions

- Work in `/Users/yelog/workspace/tui/lazydb-sqleditor` on `task/sqleditor`.
- Read `docs/plans/2026-08-26-completion-accept-cursor-design.md`.
- Do not stage the unrelated Explorer plan file.
- Do not add persistent completion-suppression state.

### Task 1: Characterize Real Cursor Desynchronization

**Files:**
- Modify: `tests/app_flow.rs`
- Modify: `src/editor/tests.rs`

**Step 1: Extend the keyword acceptance test**

After accepting `SELECT`, assert popup is none, then type one space without
leaving Insert:

```rust
assert!(app.active_console().completion.is_none());
editor_key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
assert_eq!(app.active_editor_text().unwrap(), "SELECT ");
assert_eq!(snapshot.cursor.column, 7);
```

**Step 2: Add a direct editor replacement test**

Use `replace_range(..., EndOfInsertion)` on `SELE`, then send an Insert space and
assert the result is `SELECT ` rather than ` SELECT`.

Also assert `position()` and `render_snapshot.cursor` are both column 6 before
typing.

**Step 3: Run and observe failures**

```bash
cargo test --test app_flow accepting_completion_places_cursor_after_inserted_text -- --nocapture
cargo test editor::tests::replacement_cursor_controls_next_insert_position --lib -- --nocapture
```

Expected: popup may reopen and the next Insert character uses column zero.

**Step 4: Commit red tests**

```bash
git add tests/app_flow.rs src/editor/tests.rs
git commit -m "test(completion): characterize accepted cursor state"
```

### Task 2: Synchronize Programmatic Replacement Leader

**Files:**
- Modify: `src/editor/mod.rs:683-720`
- Modify: `src/editor/mod.rs:500-600`
- Modify: `src/editor/tests.rs`

**Step 1: Set the real leader after text replacement**

Hold the buffer write guard, then:

```rust
buffer.set_text(encode_editor_text(&next));
buffer.set_leader(
    session.group_id,
    modalkit::editing::cursor::Cursor::new(position.line, position.column),
);
```

Set `session.position` to the same position only after the buffer update succeeds.

**Step 2: Derive render cursor from the leader**

In `render_snapshot_with_dialect`, read `buffer.get_leader(group_id)` and use it
for `cursor`, `cursor_screen_cell`, and the compatibility `session.position`
projection where mutation is available.

In `current_scope`, derive the cursor byte from the leader rather than the shadow
position.

**Step 3: Run editor tests**

```bash
cargo test editor::tests --lib
```

Expected: PASS, including the new continued-insert test.

**Step 4: Commit**

```bash
git add src/editor/mod.rs src/editor/tests.rs
git commit -m "fix(editor): synchronize replacement cursor leader"
```

### Task 3: Suppress Completion Refresh for Acceptance

**Files:**
- Modify: `src/app.rs:2119-2190,2310-2324`
- Modify: `tests/app_flow.rs`

**Step 1: Add a local editor-effect policy**

```rust
#[derive(Clone, Copy)]
enum EditorEffectPolicy {
    Normal,
    SuppressCompletionRefresh,
}
```

Refactor `apply_editor_effects` to delegate to a policy-taking method. For a
Changed effect under suppression:

- do not call `complete_now`;
- do not enqueue `ScheduleCompletion`;
- keep completion `None`;
- preserve all other effect handling.

**Step 2: Use suppression only in `accept_completion`**

Normal key and paste paths continue to call the normal policy. After
`replace_range`, `accept_completion` drains effects with suppression.

Do not suppress future user edits.

**Step 3: Run App tests**

```bash
cargo test --test app_flow -- --nocapture
```

Expected: `SELECT` acceptance leaves popup closed and the next space appends.

**Step 4: Add quoted candidate coverage**

Use a catalog fixture or directly install a completion candidate whose insertion
is `"sys_config"`. Assert acceptance and a following space produce:

```text
SELECT * FROM tools."sys_config" 
```

with popup none.

**Step 5: Commit**

```bash
git add src/app.rs tests/app_flow.rs
git commit -m "fix(completion): end session after acceptance"
```

### Task 4: Complete Regression Gates

**Files:**
- Modify only files required by discovered regressions

**Step 1: Run focused suites**

```bash
cargo test --test app_flow --test sql_completion --test keymap
cargo test editor::tests --lib
```

**Step 2: Run complete gates**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

**Step 3: Commit any final regression-only changes**

```bash
git add <only changed source and test files>
git commit -m "test(completion): cover accepted cursor lifecycle"
```

## Acceptance Checklist

- `SELE` -> accept `SELECT` -> popup none.
- Typing space next produces `SELECT `.
- Displayed cursor and next insertion point are identical.
- Quoted catalog candidates behave the same way.
- Acceptance remains undoable.
- Future user input can trigger completion normally.
- All complete gates pass.
