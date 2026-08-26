# SQL Editor Runtime Context Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make Vim editing reliable, make MANUAL transactions safe and discoverable, bind every SQL editor to a persisted profile/database/schema target, and restore the complete workspace across launches.

**Architecture:** Preserve `Action -> App::update -> Command -> Runtime -> DatabaseConnection` and one active runtime pool. Make one per-console modalkit machine authoritative for editing, validate pinned transaction workers by complete identity, represent the execution target in every console and execution draft, and persist a versioned manifest plus atomic SQL sidecars.

**Tech Stack:** Rust 1.94, modalkit 0.0.25, Ratatui 0.30, Crossterm 0.29, Tokio 1.47, SQLx 0.9 PostgreSQL/MySQL/SQLite adapters, Serde/TOML, existing reducer and integration-test infrastructure.

---

## Execution Preconditions

- Read `docs/plans/2026-08-26-sql-editor-runtime-context-design.md` before starting.
- Complete or checkpoint the in-progress Database Explorer implementation first. This plan depends on its `src/identity.rs`, hierarchical catalog scope, stable catalog identities, and profile-root Explorer state.
- Execute this plan in a dedicated worktree based on a clean commit containing the Database Explorer work and design commit `a4cfd0d`.
- Re-audit file line numbers after the Explorer merge. Exact symbols and responsibilities are authoritative when line numbers move.
- Keep `App::update` as the only application-state mutation boundary.
- Do not add fallback behavior that executes against the global active connection when an editor target is missing or mismatched.
- Do not persist secrets, results, output, selection, undo history, active transaction state, or connection generations.
- Follow TDD for every task: add a focused failing test, observe the expected failure, implement the minimum coherent change, run focused tests, then broader regression tests.
- Commit commands are logical checkpoints. Run them only when commit authorization is present.
- Do not begin Stage 3 transaction UI work until every Stage 2 safety test passes.

## Shared Invariants

- A console has exactly one authoritative modal state and cursor.
- Modalkit types do not escape `src/editor/`.
- Registers and command history may be shared; mode, pending keys, selection, repeat, history, and viewport are per console.
- Every logical text edit increments document revision exactly once and emits one `EditorEffect::Changed`.
- Every async result carries enough identity to reject stale console, target, connection, query, transaction, and catalog generations.
- Transaction terminal states remove the matching runtime registry entry.
- A visible transaction exit choice and the dispatched operation are always identical.
- The displayed editor target and the actual runtime target must match before execution.
- Workspace writes are private, atomic, and single-writer.

## Stage 1: Editor Authority

### Task 1: Add Full-Pipeline Editor Characterization Tests

**Files:**
- Modify: `tests/keymap.rs`
- Modify: `tests/app_flow.rs`
- Modify: `src/editor/tests.rs`

**Step 1: Add failing production-path mode tests**

In `tests/app_flow.rs`, add a helper that dispatches `KeyEvent` through
`Action::EditorKey` and assert real outcomes:

```rust
fn editor_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.update(Action::EditorKey(KeyEvent::new(code, modifiers)));
}

#[test]
fn normal_mode_motions_do_not_insert_literal_keys() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("one two\nthree".into()));

    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    for code in [
        KeyCode::Char('l'),
        KeyCode::Char('w'),
        KeyCode::Char('b'),
        KeyCode::Char('0'),
        KeyCode::Char('$'),
    ] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }

    assert_eq!(app.active_editor_text().unwrap(), "one two\nthree");
    assert_eq!(app.active_editor_mode(), EditorMode::Normal);
}
```

Add separate tests for:

- `aX<Esc>h` followed by another Normal motion;
- `ciwreplacement<Esc>`;
- `daw`, `diW`, and `dip`;
- `v`, `V`, and `Ctrl-v`, followed by `Esc`;
- Insert text followed by `u` and `Ctrl-r` through the production path;
- `f`, `F`, `t`, `T`, `gg`, and `G`;
- switching tabs while one tab has a pending `d` or `g` prefix.

**Step 2: Run and record failures**

Run:

```bash
cargo test --test app_flow normal_mode_motions_do_not_insert_literal_keys -- --nocapture
cargo test --test app_flow vim_ -- --nocapture
```

Expected: failures showing literal insertion, incorrect mode, stale cursor, or
prefix leakage. Do not weaken expected Vim behavior to match the current bug.

**Step 3: Add focused modalkit adapter tests**

In `src/editor/tests.rs`, add direct tests for mode, cursor, selection shape,
revision, and one `Changed` effect per logical edit. Keep these tests in addition
to the production-path tests; they diagnose the adapter boundary but do not
replace full-pipeline coverage.

**Step 4: Commit the red characterization tests**

```bash
git add tests/app_flow.rs tests/keymap.rs src/editor/tests.rs
git commit -m "test(editor): characterize modal input failures"
```

### Task 2: Make Modal State Per Console and Authoritative

**Files:**
- Modify: `src/editor/mod.rs:141-215` (`EditorSession`, `EditorWorkspace`)
- Modify: `src/editor/mod.rs:218-269` (open and key input)
- Modify: `src/editor/mod.rs:659-1010` (press, action draining, synchronization)
- Modify: `src/editor/tests.rs`

**Step 1: Add a failing console-isolation test**

```rust
#[test]
fn consoles_do_not_share_mode_or_pending_prefixes() {
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let mut editor = EditorWorkspace::new();
    editor.open_console(first, "one");
    editor.open_console(second, "two");

    editor.press(first, EditorKey::Escape).unwrap();
    editor.press(first, EditorKey::Character('d')).unwrap();
    editor.press(second, EditorKey::Character('i')).unwrap();
    editor.press(second, EditorKey::Character('X')).unwrap();

    assert_eq!(editor.text(first).unwrap(), "one");
    assert_eq!(editor.text(second).unwrap(), "Xtwo");
    assert_eq!(editor.mode(first).unwrap(), EditorMode::Normal);
    assert_eq!(editor.mode(second).unwrap(), EditorMode::Insert);
}
```

**Step 2: Run the focused test**

Run: `cargo test editor::tests::consoles_do_not_share_mode_or_pending_prefixes -- --nocapture`

Expected: FAIL because `KeyManager`, pending bindings, and current sequence are
workspace-global.

**Step 3: Move modal machine state into `EditorSession`**

Replace shadow fields and workspace-global machines with a per-session machine:

```rust
type VimKeyManager = modalkit::editing::key::KeyManager<
    modalkit::key::TerminalKey,
    modalkit::actions::Action<LazyDbApplicationInfo>,
    modalkit::prelude::RepeatType,
>;

struct EditorSession {
    buffer: SharedBuffer<LazyDbApplicationInfo>,
    group_id: CursorGroupId,
    viewport: ViewportContext<Cursor>,
    keys: VimKeyManager,
    pending_binding: Option<PendingBinding>,
    current_sequence: Vec<EditorKey>,
    last_sequence: Option<Vec<EditorKey>>,
    revision: u64,
}
```

Remove `mode`, `position`, `previous_text`, and `redo_text` as authorities. Derive
mode and cursor from the session's key manager and buffer leader. Initialize the
new console machine in Insert using the same modalkit action path used for later
mode changes; do not assign a display-only Insert value.

Keep `Store` shared for registers. If modalkit requires store-owned history for a
buffer, verify that buffer IDs and cursor groups remain console-specific.

**Step 4: Route every non-prompt editor key through the session machine**

Remove manual branches for `Esc`, `i`, printable Insert characters, backspace,
and control editing. Keep only application-owned prompt routing outside modalkit.
Register application leader bindings in the modalkit binding set where possible;
if `PendingBinding` remains, keep it inside the active session and never infer
operation-pending state from `show_mode()`.

**Step 5: Run isolation and mode tests**

Run:

```bash
cargo test editor::tests::consoles_do_not_share_mode_or_pending_prefixes
cargo test --test app_flow normal_mode_motions_do_not_insert_literal_keys
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/editor/mod.rs src/editor/tests.rs tests/app_flow.rs
git commit -m "fix(editor): make modal state console-local"
```

### Task 3: Use Modalkit Edits and Complete Action Draining

**Files:**
- Modify: `src/editor/mod.rs:716-1019`
- Modify: `src/editor/tests.rs`
- Modify: `src/app.rs:520-534,1939-1988`
- Test: `tests/app_flow.rs`

**Step 1: Add failing revision and history tests**

Add tests that type one Insert session, execute `u`, execute `Ctrl-r`, and assert:

- text returns through the expected states;
- revision increments once per logical undoable edit, not once per byte;
- every changed revision emits exactly one `EditorEffect::Changed`;
- completion/highlighting schedules use the new revision.

Add tests for modalkit `Repeat`, viewport scroll, jump, and macro actions. Tests
must fail if an action becomes a generic `"deferred"` message.

**Step 2: Observe failures**

Run: `cargo test editor::tests::vim_undo_redo_and_dot_repeat_are_available -- --nocapture`

Expected: ordinary Insert changes are outside modalkit history and some action
classes are deferred.

**Step 3: Remove whole-buffer incremental editing**

Delete manual `insert`, `backspace`, `replace_at_cursor`, `previous_text`, and
`redo_text` paths used for normal key input. Use modalkit insert/edit actions and
history checkpoints. Keep a separate programmatic replacement API for load,
format, substitution, and completion.

Capture text and revision before processing one input event. After draining all
actions, compare the authoritative buffer and finalize once:

```rust
fn finalize_input(&mut self, id: Uuid, before: EditorFingerprint) -> Result<()> {
    let after = self.fingerprint(id)?;
    if before.text != after.text {
        let revision = self.bump_revision(id)?;
        self.effects.push(EditorEffect::Changed { console_id: id, revision });
    }
    Ok(())
}
```

Do not use the full text as the long-term fingerprint if profiling shows it is
expensive; correctness comes first, then replace it with modalkit change metadata.

**Step 4: Handle supported action classes**

Follow modalkit-ratatui's integration pattern for `Editor`, `Repeat`, `Scroll`,
`Jump`, and `Macro`. Feed repeat actions back into the active session's binding
machine. Translate application actions to `EditorEffect`. Return a visible
`EditorEffect::Message` only for a genuinely unsupported action, including its
class and debug value.

In `App::apply_editor_effects`, surface editor errors and unsupported actions in
the active console output instead of discarding them.

**Step 5: Run editor regressions**

Run:

```bash
cargo test editor::tests -- --nocapture
cargo test --test app_flow -- --nocapture
cargo test --test keymap -- --nocapture
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/editor/mod.rs src/editor/tests.rs src/app.rs tests/app_flow.rs
git commit -m "fix(editor): unify edits and modal history"
```

### Task 4: Render Exact Selections and Mode Cursor Styles

**Files:**
- Modify: `src/model/editor.rs`
- Modify: `src/editor/mod.rs:451-493`
- Modify: `src/ui/mod.rs:529-630`
- Modify: `src/terminal.rs`
- Modify: `src/runtime.rs:1506-1590`
- Test: `tests/ui_render.rs`
- Test: `tests/app_flow.rs`

**Step 1: Add failing render-projection tests**

Create snapshots for Visual Char, Line, and Block over ASCII, CJK, emoji, and tab
content. Assert exact selected display cells rather than complete selected lines.

Add a domain cursor style assertion:

```rust
assert_eq!(state.cursor_style, Some(CursorStyle::Block));
```

Repeat for Insert (`Bar`), Replace, Visual, prompt, non-editor focus, and overlay.

**Step 2: Run focused UI tests**

Run: `cargo test --test ui_render editor_cursor -- --nocapture`

Expected: FAIL because selection columns and cursor style are not represented.

**Step 3: Add display-cell selection projection**

Extend `EditorRenderSnapshot` with a selection projection that is already clipped
to the viewport and expressed in terminal cells. Keep raw offsets inside the
editor module. Render each selected cell interval with the existing selection
style.

**Step 4: Add terminal cursor style effects**

Define a domain enum in `src/ui/mod.rs` or `src/model/editor.rs`:

```rust
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}
```

Add `cursor_style: Option<CursorStyle>` to `UiState`. After each draw,
`TerminalSession` applies `SetCursorStyle::SteadyBlock`, `SteadyBar`, or
`SteadyUnderScore` only when the requested style changes. Restore
`SetCursorStyle::DefaultUserShape` in `Drop` and `restore_terminal()`.

**Step 5: Run focused and terminal tests**

Run:

```bash
cargo test --test ui_render -- --nocapture
cargo test --test app_flow visual_ -- --nocapture
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/model/editor.rs src/editor/mod.rs src/ui/mod.rs src/terminal.rs src/runtime.rs tests/ui_render.rs tests/app_flow.rs
git commit -m "feat(editor): render modal cursor and selections"
```

### Task 5: Fix Programmatic Replacement and Completion Cursor

**Files:**
- Modify: `src/editor/mod.rs:579-610`
- Modify: `src/app.rs:2027-2038,2082-2089`
- Modify: `tests/sql_completion.rs`
- Modify: `tests/app_flow.rs`

**Step 1: Add a failing completion acceptance test**

Through `App::update`, type `sel`, install candidates, accept `SELECT`, type one
space, and assert:

```rust
assert_eq!(app.active_editor_text().unwrap(), "SELECT ");
assert_eq!(app.active_editor_cursor_byte(), 7);
```

Then send `Esc`, `u`, and assert the accepted completion is undone as one logical
edit according to the chosen undo checkpoint.

**Step 2: Run the focused test**

Run: `cargo test --test app_flow completion_accept -- --nocapture`

Expected: FAIL because replacement places the cursor at `range.start`.

**Step 3: Add explicit cursor policy**

Add:

```rust
pub(crate) enum ReplacementCursor {
    Start,
    EndOfInsertion,
    PreserveRelative,
}
```

Change `replace_range` to accept the policy, perform an editor-native replacement,
set the real buffer leader, and create one history entry. Completion uses
`EndOfInsertion`; formatting and substitution state their policy explicitly.

**Step 4: Run completion and editor tests**

Run:

```bash
cargo test --test app_flow completion_accept -- --nocapture
cargo test --test sql_completion -- --nocapture
cargo test editor::tests -- --nocapture
```

Expected: PASS.

**Step 5: Run Stage 1 gate**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --test keymap --test app_flow --test ui_render --test sql_completion
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/editor/mod.rs src/app.rs tests/app_flow.rs tests/sql_completion.rs
git commit -m "fix(completion): place cursor after accepted text"
```

## Stage 2: Transaction Safety

### Task 6: Make Exit Choices Typed and Exact

**Files:**
- Modify: `src/action.rs:122-126`
- Modify: `src/input/keymap.rs:77-97`
- Modify: `src/model/transaction.rs:43-80`
- Modify: `src/app.rs:1313-1415`
- Modify: `src/ui/mod.rs:941-1003`
- Test: `tests/transaction_reducer.rs`
- Test: `tests/keymap.rs`
- Test: `tests/ui_render.rs`

**Step 1: Add failing exit-choice tests**

Cover Enter on default Rollback, explicit `r`, explicit `c`, Tab then Enter, and
Escape. Assert both the visible choice and resulting `Command::ManualRollback` or
`Command::ManualCommit`.

**Step 2: Observe the dangerous failure**

Run: `cargo test --test transaction_reducer transaction_exit -- --nocapture`

Expected: FAIL because every confirmation currently reduces as Commit.

**Step 3: Carry the selected choice in the Action**

Replace the choice-free confirm action with either:

```rust
Action::ConfirmTransactionExit(TransactionExitChoice)
```

or make the reducer read the overlay's current typed choice before removing it.
Map `r` and `c` to explicit typed actions. Enter dispatches the current overlay
selection. Escape dispatches Cancel and clears every prompt associated with the
same deferred intent.

For OUTCOME_UNKNOWN, never render Commit/Rollback exit options; route to the
verification flow.

**Step 4: Run focused tests**

Run:

```bash
cargo test --test transaction_reducer transaction_exit
cargo test --test keymap transaction_exit
cargo test --test ui_render transaction_exit
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/action.rs src/input/keymap.rs src/model/transaction.rs src/app.rs src/ui/mod.rs tests/transaction_reducer.rs tests/keymap.rs tests/ui_render.rs
git commit -m "fix(transaction): honor exit confirmation choices"
```

### Task 7: Route Transaction SQL Through Immutable Drafts

**Files:**
- Modify: `src/sql/execution.rs`
- Modify: `src/sql/transaction.rs`
- Modify: `src/app.rs:2171-2384`
- Modify: `tests/sql_execution.rs`
- Modify: `tests/transaction_sql.rs`

**Step 1: Add failing scope and running-query tests**

Use a buffer containing data SQL plus a current-scope savepoint. Assert only the
exact savepoint scope enters `Command::ManualExecute`. Add tests that BEGIN,
COMMIT, ROLLBACK, and savepoint controls cannot bypass `QueryStatus::Running`.

**Step 2: Run tests**

Run: `cargo test --test sql_execution transaction_control -- --nocapture`

Expected: FAIL because savepoints reread the full editor and controls branch
before the running-query check.

**Step 3: Preserve control and exact SQL in `ExecutionDraft`**

Extend the draft with a classified execution kind:

```rust
enum ExecutionKind {
    Data,
    Transaction(TransactionControl),
}
```

Create every draft after resolving exact scope. Apply confirmation and stale
validation before dispatch. Remove the obsolete `"unavailable until Task 16"`
gate. Dispatch control using `draft.sql`, never `editor_text()`.

**Step 4: Run focused tests**

Run:

```bash
cargo test --test sql_execution
cargo test --test transaction_sql
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/sql/execution.rs src/sql/transaction.rs src/app.rs tests/sql_execution.rs tests/transaction_sql.rs
git commit -m "fix(transaction): preserve control execution scope"
```

### Task 8: Add Worker Readiness and Complete Runtime Identity

**Files:**
- Modify: `src/identity.rs`
- Modify: `src/db/transaction.rs`
- Modify: `src/runtime/transaction.rs`
- Modify: `src/runtime.rs:52-97,806-910,1416-1484`
- Modify: `src/action.rs:171-230,286-310`
- Test: `src/runtime/transaction.rs`
- Test: `tests/transaction_reducer.rs`

**Step 1: Add failing readiness and stale-identity tests**

With the fake backend, make `begin()` fail and assert Runtime emits
`ManualStartFailed`, never `ManualStarted`. Add a delayed begin test proving
`ManualStarted` is not emitted before acknowledgement.

Add stale execute, commit, rollback, and cancel tests for mismatched profile,
connection generation, transaction generation, and query generation.

**Step 2: Run focused tests**

Run: `cargo test runtime::transaction::tests -- --nocapture`

Expected: FAIL because worker creation returns before BEGIN and Runtime primarily
looks entries up by console ID.

**Step 3: Define complete worker identity**

Add a hashable identity:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TransactionIdentity {
    pub console_id: Uuid,
    pub connection: ConnectionIdentity,
    pub generation: u64,
}
```

Store this identity in `ManualTransactionEntry` and validate it on every request.
Do not trust command fields after looking up only by `console_id`.

**Step 4: Add a BEGIN readiness channel**

Extend `TransactionWorkerHandle` with a one-shot readiness receiver carrying
`Result<(), TransactionError>`. The worker sends success only after `begin()` and
depth validation. Runtime awaits readiness before inserting the registry entry
and emitting `ManualStarted`.

On failure, await worker termination, return `ManualStartFailed`, and do not leave
an entry.

**Step 5: Run worker and reducer tests**

Run:

```bash
cargo test runtime::transaction::tests
cargo test --test transaction_reducer
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/identity.rs src/db/transaction.rs src/runtime/transaction.rs src/runtime.rs src/action.rs tests/transaction_reducer.rs
git commit -m "fix(transaction): validate worker readiness and identity"
```

### Task 9: Remove Terminal Workers and Support Repeated Transactions

**Files:**
- Modify: `src/runtime.rs:684-910,1021-1048,1474-1484`
- Modify: `src/runtime/transaction.rs`
- Modify: `tests/sqlite_transactions.rs`
- Create or modify: `tests/transaction_runtime.rs`

**Step 1: Add a failing end-to-end SQLite runtime test**

Run this sequence through App and Runtime, not direct SQLx connection calls:

```text
MANUAL
INSERT first row
ROLLBACK
MANUAL
INSERT second row
COMMIT
SELECT
```

Assert only the second row exists and the second transaction starts in the same
console. Add variants for explicit `ManualBegin` and lazy first execution.

**Step 2: Observe failure**

Run: `cargo test --test transaction_runtime repeated_manual_transactions -- --nocapture`

Expected: FAIL because a terminal worker entry can remain registered.

**Step 3: Make the registry own terminal cleanup**

Use one Runtime-owned proxy task per entry. On `WorkerDisposition`, remove only
the matching `TransactionIdentity` entry before emitting the terminal Action.
Avoid opportunistic `is_finished()` cleanup as the correctness mechanism.

Store the real worker `JoinHandle` and real forced-close handle. Shutdown sends
`Shutdown`, awaits a bounded rollback, then aborts and awaits the actual physical
worker before closing pools.

**Step 4: Run runtime tests**

Run:

```bash
cargo test --test transaction_runtime -- --nocapture
cargo test runtime::transaction::tests -- --nocapture
cargo test --test sqlite_transactions -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/runtime.rs src/runtime/transaction.rs tests/transaction_runtime.rs tests/sqlite_transactions.rs
git commit -m "fix(transaction): retire terminal workers safely"
```

### Task 10: Repair Cancellation and Structured Outcomes

**Files:**
- Modify: `src/db/transaction.rs`
- Modify: `src/runtime/transaction.rs`
- Modify: `src/runtime.rs`
- Modify: `src/model/transaction.rs`
- Modify: `src/app.rs:592-647,955-1122,2105-2169`
- Modify: `src/action.rs`
- Test: `src/runtime/transaction.rs`
- Test: `tests/transaction_reducer.rs`
- Test: `tests/transaction_runtime.rs`

**Step 1: Add failing cancellation and outcome tests**

Cover cancellation in Starting and Active, cancellation acknowledgement loss,
commit connection loss, rollback connection loss, ordinary PostgreSQL statement
failure, ordinary MySQL/SQLite statement failure, and shutdown during a hung
query.

Assert that uncertain commit/rollback becomes OUTCOME_UNKNOWN, cancellation that
cannot prove rollback becomes OUTCOME_UNKNOWN, and no query listener is detached
while the worker continues unowned.

**Step 2: Run focused tests**

Run: `cargo test --test transaction_runtime cancellation -- --nocapture`

Expected: FAIL in Starting and uncertain-outcome transitions.

**Step 3: Replace string-only transaction errors**

Use a structured domain result such as:

```rust
pub struct TransactionError {
    pub category: ErrorCategory,
    pub code: Option<String>,
    pub message: String,
    pub connection_trust: ConnectionTrust,
    pub operation_sent: bool,
    pub acknowledged: bool,
}
```

Adapters preserve `DatabaseError` category and code. Runtime decides worker
disposition; App decides visible transaction state from the structured result.

**Step 4: Serialize cancel and rollback**

Do not drop a live execute future and independently race rollback. Keep the
physical worker authoritative until the query reaches a known cancelled terminal
state or the connection is quarantined. Starting cancellation cancels the begin
operation through the same owner rather than aborting only an outer listener.

Extend the transaction reducer so valid cancellation uncertainty can enter
OUTCOME_UNKNOWN from the actual cancellation state, or introduce an explicit
Cancelling state if that produces clearer invariants.

**Step 5: Preflight commit and rollback before state mutation**

Verify query idle, matching target/connection, and matching worker identity before
entering COMMITTING or ROLLING_BACK. Rejections append visible output and leave
the previous state intact.

**Step 6: Run Stage 2 gate**

Run:

```bash
cargo test runtime::transaction::tests -- --nocapture
cargo test --test transaction_reducer --test transaction_runtime --test transaction_sql --test sql_execution
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Step 7: Commit**

```bash
git add src/db/transaction.rs src/runtime/transaction.rs src/runtime.rs src/model/transaction.rs src/app.rs src/action.rs tests/transaction_reducer.rs tests/transaction_runtime.rs
git commit -m "fix(transaction): make cancellation outcomes truthful"
```

## Stage 3: Transaction Controls

### Task 11: Connect Transaction Effects and Add Discoverable UI

**Files:**
- Modify: `src/editor/mod.rs` (application bindings and Ex effects)
- Modify: `src/app.rs:1939-1988`
- Modify: `src/action.rs`
- Modify: `src/ui/mod.rs:275-364,529-630,838-889,1099-1145`
- Modify: `src/input/mouse.rs`
- Modify: `docs/keybindings.md`
- Modify: `README.md`
- Test: `tests/app_flow.rs`
- Test: `tests/keymap.rs`
- Test: `tests/mouse.rs`
- Test: `tests/ui_render.rs`

**Step 1: Add failing full-path control tests**

Test `Space tt`, `Space tc`, `Space tr`, `:tx auto`, `:tx manual`, `:commit`,
`:rollback`, and `:tx clear` through editor effect and `App::update`. Assert
state-dependent availability and visible errors for invalid controls.

Add UI tests for every transaction state at 120x36 and 80x24. Add mouse tests for
opening the transaction menu and choosing an action.

**Step 2: Observe failures**

Run:

```bash
cargo test --test app_flow transaction_controls
cargo test --test ui_render transaction_status
```

Expected: toggle and clear effects are discarded; controls/help are missing.

**Step 3: Translate all transaction effects**

Map `ToggleTransaction` using current console state and `ClearTransactionOutcome`
to its existing confirmation action. Keyboard, Ex, and mouse paths all emit the
same semantic transaction Actions.

**Step 4: Add state projection and controls**

Create one pure availability projection consumed by reducer and UI:

```rust
struct TransactionAvailability {
    can_toggle: bool,
    can_commit: bool,
    can_rollback: bool,
    can_verify: bool,
}
```

Render textual state with warning/error styles, add a state-dependent transaction
menu and hit targets, and add Transaction help. Do not expose disabled actions as
enabled hit regions.

**Step 5: Update documentation only after tests pass**

Document the now-working keys, Ex commands, state meanings, and unknown-outcome
recovery. Remove any statements that still overpromise unsupported behavior.

**Step 6: Run Stage 3 tests**

Run:

```bash
cargo test --test app_flow --test keymap --test mouse --test ui_render
cargo test --test transaction_reducer --test transaction_runtime
```

Expected: PASS.

**Step 7: Commit**

```bash
git add src/editor/mod.rs src/app.rs src/action.rs src/ui/mod.rs src/input/mouse.rs docs/keybindings.md README.md tests/app_flow.rs tests/keymap.rs tests/mouse.rs tests/ui_render.rs
git commit -m "feat(transaction): expose safe editor controls"
```

## Stage 4: Execution Targets

### Task 12: Add the Execution Target Domain and Fail-Closed Drafts

**Files:**
- Create: `src/model/execution_target.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/identity.rs`
- Modify: `src/sql/execution.rs`
- Modify: `src/app.rs`
- Create: `tests/execution_target.rs`
- Modify: `tests/sql_execution.rs`

**Step 1: Add failing target-validation tests**

Test exact target equality, MySQL database/schema normalization, SQLite default
alias, missing profile, empty database, and out-of-scope schema. Test that a draft
is rejected if the tab target changes after confirmation or if Runtime is
connected to a different target.

**Step 2: Run focused tests**

Run: `cargo test --test execution_target -- --nocapture`

Expected: FAIL because consoles and drafts have no target.

**Step 3: Add serializable target types**

```rust
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ExecutionTarget {
    pub profile_id: Uuid,
    pub database: String,
    pub schema: Option<String>,
}
```

Add validation against `ConnectionProfile`, database kind, and `CatalogScope`.
Construct new-console defaults from the selected or active profile. Allow a
distinct unresolved target state for first launch or missing profiles; never use
an all-zero UUID or empty database as a valid target.

Add the exact target to `ConsoleTab` and `ExecutionDraft`. Extend active runtime
identity with the target database and connection generation. Include schema in
the connection-session target comparison where it changes session behavior.

**Step 4: Make execution fail closed**

Before draft creation and again before dispatch, validate the console target,
active runtime target, and connection identity. Return a visible error on
OFFLINE, LINKING, MISSING PROFILE, INVALID TARGET, or mismatch.

**Step 5: Run tests**

Run:

```bash
cargo test --test execution_target
cargo test --test sql_execution
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/model/execution_target.rs src/model/mod.rs src/model/tab.rs src/identity.rs src/sql/execution.rs src/app.rs tests/execution_target.rs tests/sql_execution.rs
git commit -m "feat(sql): bind consoles to execution targets"
```

### Task 13: Safely Auto-Switch Targets When Activating Tabs

**Files:**
- Modify: `src/action.rs`
- Modify: `src/model/transaction.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/app.rs:163-180` and connection lifecycle handlers
- Modify: `src/runtime.rs`
- Modify: `tests/connection_switch.rs`
- Modify: `tests/execution_target.rs`

**Step 1: Add failing activation tests**

Cover:

- same target activates immediately;
- different target records pending activation and keeps the old active tab;
- matching connection success completes activation;
- failed connection retains old tab and old pool;
- stale success cannot activate a tab;
- running query blocks switching;
- active transaction invokes the lifecycle prompt;
- deleting the pending target profile cancels activation.

**Step 2: Run focused tests**

Run: `cargo test --test execution_target activate_tab -- --nocapture`

Expected: FAIL because `ActivateTab` changes `active_tab` immediately.

**Step 3: Add pending activation state**

Introduce a domain request containing target tab ID, expected target, and
connection request generation. Reuse the existing safe-switch connection flow.
Only a matching `ConnectionSucceeded` may commit `active_tab`.

Extend `DeferredIntent` with target activation. Ensure cancelling any transaction
prompt cancels the complete activation intent.

**Step 4: Run switch and lifecycle tests**

Run:

```bash
cargo test --test execution_target activate_tab
cargo test --test connection_switch
cargo test --test transaction_reducer deferred
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/action.rs src/model/transaction.rs src/model/workspace.rs src/app.rs src/runtime.rs tests/connection_switch.rs tests/execution_target.rs tests/transaction_reducer.rs
git commit -m "feat(sql): switch connection with active console"
```

### Task 14: Implement Backend-Specific Database and Schema Context

**Files:**
- Modify: `src/db/mod.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `src/runtime.rs`
- Modify: `tests/postgres_adapter.rs`
- Modify: `tests/mysql_adapter.rs`
- Modify: `tests/sqlite_adapter.rs`
- Modify: `tests/execution_target.rs`

**Step 1: Add failing adapter contract tests**

PostgreSQL tests assert database changes build a new pool target and every pooled
connection receives the selected schema search path. Use driver-safe option APIs
or a safely quoted `after_connect` statement; never concatenate raw identifiers.

MySQL tests assert target schema equals database and connect options select that
database without per-query `USE`.

SQLite tests assert `main`, `temp`, and discovered attached aliases are accepted,
while unknown aliases are rejected.

**Step 2: Run adapter tests**

Run:

```bash
cargo test --test postgres_adapter execution_target -- --nocapture
cargo test --test mysql_adapter execution_target -- --nocapture
cargo test --test sqlite_adapter execution_target -- --nocapture
```

Expected: FAIL because adapters only consume profile defaults.

**Step 3: Pass validated target into connection creation**

Change `DatabaseConnection::connect` and concrete adapters to accept a validated
connection target. PostgreSQL sets database and schema initialization. MySQL sets
database. SQLite validates alias after discovery and does not create a second
file connection for schema changes.

Make Runtime's `ActiveConnection` store the exact target projection returned by
the adapter probe. App treats any normalized target difference as stale.

**Step 4: Run adapter and conditional integration tests**

Run:

```bash
cargo test --test postgres_adapter --test mysql_adapter --test sqlite_adapter
```

When configured, also run:

```bash
cargo test --test postgres_adapter connects_and_decodes_common_postgres_values_when_configured -- --nocapture
cargo test --test mysql_adapter connects_and_decodes_common_mysql_values_when_configured -- --nocapture
```

Expected: PASS or documented skip when environment variables are absent.

**Step 5: Commit**

```bash
git add src/db/mod.rs src/db/postgres.rs src/db/mysql.rs src/db/sqlite.rs src/runtime.rs tests/postgres_adapter.rs tests/mysql_adapter.rs tests/sqlite_adapter.rs tests/execution_target.rs
git commit -m "feat(db): apply editor execution targets"
```

### Task 15: Add Target Selector, Ex Commands, and Target-Local Completion

**Files:**
- Modify: `src/action.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/editor/mod.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/sql/completion.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/mouse.rs`
- Modify: `tests/ui_render.rs`
- Modify: `tests/sql_completion.rs`
- Modify: `tests/execution_target.rs`

**Step 1: Add failing selector and completion tests**

Test status display, READY/LINKING/OFFLINE/FAILED text, keyboard selectors,
`:connection`, `:database`, `:schema`, mouse activation, Escape cancellation, and
Enter confirmation. Assert profile changes choose a valid default and database
changes reset schema.

Add completion tests proving two tabs on different schemas rank their own schema
objects first and stale catalog generations cannot update the wrong target.

**Step 2: Run focused tests**

Run:

```bash
cargo test --test execution_target selector -- --nocapture
cargo test --test sql_completion target -- --nocapture
```

Expected: FAIL because no selector actions or target-local completion exist.

**Step 3: Add semantic target-selection actions**

Use one overlay state with Connection, Database, and Schema levels. Candidate
values come from profiles and the active profile's stable Explorer catalog. Do
not duplicate catalog discovery in UI code.

Register:

```text
Space d c
Space d d
Space d s
```

and parse the three Ex commands to the same semantic actions. Explorer "set as
target" operations use those actions too.

**Step 4: Use the console target for completion**

Build `CompletionScheduleKey` from complete target and catalog generation. Pass
the tab's selected schema as the default schema, not the profile's global default.
Reject due results after any target or catalog identity change.

**Step 5: Run Stage 4 gate**

Run:

```bash
cargo test --test execution_target --test sql_completion --test keymap --test mouse --test ui_render
cargo test --test connection_switch
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/action.rs src/model/workspace.rs src/editor/mod.rs src/app.rs src/ui/mod.rs src/input/keymap.rs src/input/mouse.rs src/sql/completion.rs tests/keymap.rs tests/mouse.rs tests/ui_render.rs tests/sql_completion.rs tests/execution_target.rs
git commit -m "feat(sql): add execution target selector"
```

## Stage 5: Workspace Persistence

### Task 16: Add Versioned Workspace Manifest and SQL Sidecars

**Files:**
- Create: `src/persistence/workspace.rs`
- Modify: `src/persistence/mod.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/model/transaction.rs`
- Create: `tests/workspace_persistence.rs`

**Step 1: Add failing round-trip tests**

Create fixtures with multiple consoles, stable UUIDs, order, active console,
Unicode and large SQL, targets, and AUTO/MANUAL preferences. Assert MANUAL restores
as Idle and excluded transient fields are absent from manifest text.

Add missing SQL sidecar, missing profile, invalid target, unsupported version, and
orphan sidecar cases.

**Step 2: Run focused tests**

Run: `cargo test --test workspace_persistence round_trip -- --nocapture`

Expected: FAIL because `WorkspaceStore` does not exist.

**Step 3: Implement persistence-only snapshots**

Define serializable manifest records separate from `ConsoleTab`:

```rust
struct WorkspaceFile {
    version: u16,
    active_console: Uuid,
    consoles: Vec<PersistedConsole>,
}

struct PersistedConsole {
    id: Uuid,
    name: String,
    sql_file: PathBuf,
    target: Option<ExecutionTarget>,
    transaction_mode: TransactionMode,
}
```

Do not derive serialization on the entire live App or ConsoleTab.

Write SQL temporary files first, sync, rename, then write and rename the manifest.
Use the same private directory/file permissions as profile persistence. Clean
orphans only after manifest success.

**Step 4: Run persistence tests**

Run: `cargo test --test workspace_persistence -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/persistence/workspace.rs src/persistence/mod.rs src/model/tab.rs src/model/transaction.rs tests/workspace_persistence.rs
git commit -m "feat(workspace): persist console manifests and sql"
```

### Task 17: Add Single-Writer Locking and Read-Only Recovery

**Files:**
- Modify: `Cargo.toml` and `Cargo.lock` only if a proven cross-platform locking dependency is required
- Modify: `src/persistence/workspace.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/ui/mod.rs`
- Modify: `tests/workspace_persistence.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Add failing lock tests**

Open the same workspace twice. Assert the first store is writable and the second
loads the same content in explicit read-only mode. Assert the second cannot
overwrite files and UI renders `WORKSPACE READ ONLY` without relying on color.

**Step 2: Run focused tests**

Run: `cargo test --test workspace_persistence lock -- --nocapture`

Expected: FAIL because no locking exists.

**Step 3: Implement a lifetime-held lock**

Prefer a standard-library or already-transitive mechanism only if its semantics
are reliable on supported platforms. If a dependency is necessary, use a small,
maintained advisory-lock crate and document why create-new lockfiles alone cannot
recover safely after crashes.

Hold the lock handle for the Runtime lifetime. Expose writable/read-only status to
App through startup state. Unsupported-newer manifests also open read-only.

**Step 4: Run lock and UI tests**

Run:

```bash
cargo test --test workspace_persistence lock
cargo test --test ui_render workspace_read_only
```

Expected: PASS.

**Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/persistence/workspace.rs src/model/workspace.rs src/ui/mod.rs tests/workspace_persistence.rs tests/ui_render.rs
git commit -m "feat(workspace): lock persistence to one writer"
```

### Task 18: Connect Debounced Save, Shutdown Flush, and Startup Restore

**Files:**
- Modify: `src/action.rs:336`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs:84-105,257-258,1506-1518`
- Modify: `src/cli.rs` if workspace path injection is needed for tests
- Modify: `tests/app_flow.rs`
- Modify: `tests/startup_profiles.rs`
- Modify: `tests/workspace_persistence.rs`

**Step 1: Add failing runtime persistence tests**

Test that SQL edits, target changes, tab changes, active-tab changes, and mode
preference changes schedule persistence. Rapid SQL edits coalesce. Clean quit
flushes immediately. Save failure creates visible output but keeps text in memory.

Test startup restoration of order, IDs, SQL, active tab, target, and MANUAL:IDLE,
followed by safe connection of the restored active tab.

**Step 2: Run focused tests**

Run:

```bash
cargo test --test workspace_persistence debounce -- --nocapture
cargo test --test app_flow workspace -- --nocapture
```

Expected: FAIL because `PersistWorkspace` is a no-op.

**Step 3: Carry immutable workspace snapshots in commands**

Do not let Runtime read mutable App state. Replace the empty command with a
snapshot or a generation-keyed scheduling command:

```rust
Command::PersistWorkspace {
    generation: u64,
    snapshot: WorkspaceSnapshot,
}
```

Runtime coalesces snapshots for 300-500 ms, writes only the latest generation,
and sends success/failure actions. Shutdown cancels debounce and flushes the latest
snapshot before terminal and runtime teardown complete.

**Step 4: Restore before constructing live App state**

Load profiles and workspace during startup. Resolve persisted targets against
profiles without discarding invalid consoles. Build `EditorWorkspace` sessions
using persisted IDs and SQL. Attempt connection only after App and Runtime agree
on the restored active target.

**Step 5: Run Stage 5 tests**

Run:

```bash
cargo test --test workspace_persistence --test app_flow --test startup_profiles
cargo test --test execution_target --test connection_switch
```

Expected: PASS.

**Step 6: Commit**

```bash
git add src/action.rs src/app.rs src/runtime.rs src/cli.rs tests/app_flow.rs tests/startup_profiles.rs tests/workspace_persistence.rs
git commit -m "feat(workspace): restore and autosave sql consoles"
```

## Final Integration

### Task 19: Update Documentation and Run Complete Acceptance Gates

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/keybindings.md`
- Modify: relevant tests discovered during full regression

**Step 1: Add one cross-feature acceptance flow**

Create a test using SQLite that:

1. Restores two targeted consoles.
2. Uses real Normal and Insert keys.
3. Accepts completion and verifies cursor placement.
4. Switches to MANUAL.
5. Executes and rolls back.
6. Starts another transaction and commits.
7. Switches tabs and target safely.
8. Flushes and restores the workspace.

Keep lower-level tests; this flow validates integration boundaries, not every
edge case.

**Step 2: Run the cross-feature test**

Run: `cargo test --test app_flow sql_editor_runtime_context -- --nocapture`

Expected: PASS.

**Step 3: Update documentation to observed behavior**

Document:

- exact supported Vim operations and cursor styles;
- target selector keys and Ex commands;
- AUTO/MANUAL state semantics;
- unknown-outcome recovery;
- workspace files, lock behavior, and restored/non-restored data;
- one-active-pool tab-switch behavior.

Do not document any item whose production-path test is absent or failing.

**Step 4: Run complete local gates**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

Expected: PASS.

**Step 5: Run configured database integrations**

If `LAZYDB_TEST_POSTGRES_URL` is set:

```bash
cargo test --test postgres_adapter -- --nocapture
```

If `LAZYDB_TEST_MYSQL_URL` is set:

```bash
cargo test --test mysql_adapter -- --nocapture
```

Record skipped integrations explicitly when environment variables are absent.

**Step 6: Perform manual TUI acceptance**

Verify on a real terminal at wide and compact sizes:

```text
Normal/Insert/Visual cursor shapes
h/j/k/l, words, line motions, find motions
operators and text objects
Visual Char/Line/Block selection cells
undo and redo
completion acceptance and continued typing
transaction menu and typed confirmations
target selector and failed auto-switch
workspace restart and read-only lock warning
```

**Step 7: Commit documentation and final test adjustments**

```bash
git add README.md docs/architecture.md docs/keybindings.md tests
git commit -m "docs(sql): document editor runtime context"
```

## Completion Checklist

- Normal-mode keys never insert literal command characters.
- Documented motions, operators, text objects, Visual modes, undo, and redo pass
  through the production input pipeline.
- Cursor shape follows the authoritative editor mode.
- Completion cursor lands after insertion and completion is undoable.
- Transaction exit defaults and typed choices are exact.
- Repeated MANUAL transactions work in one console.
- BEGIN, cancellation, commit, rollback, and shutdown outcomes are truthful.
- Transaction controls are visible and state-dependent.
- Every console owns a validated profile/database/schema target.
- Tab activation safely switches the one active pool.
- Drafts reject every target or identity mismatch.
- Completion and catalog results cannot cross targets.
- Workspace manifest and SQL sidecars restore complete editor state.
- MANUAL restores as Idle and no active transaction is persisted.
- Workspace locking prevents concurrent writers.
- All local gates and configured database integrations pass.
