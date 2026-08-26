# SQL Editor Context Title Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move transaction and execution-target status from the global Header to a responsive right-aligned SQL Editor title and document the controls in Help.

**Architecture:** Add one pure UI projection for target and transaction labels. Render two native Ratatui top titles on the SQL Editor block, remove editor-local data from the Header, and select title detail based on available width with transaction priority.

**Tech Stack:** Rust 1.94, Ratatui 0.30 multiple Block titles, existing UI snapshot tests.

---

## Preconditions

- Work in `/Users/yelog/workspace/tui/lazydb-sqleditor` on `task/sqleditor`.
- Read `docs/plans/2026-08-26-editor-context-title-design.md`.
- Do not stage the unrelated Explorer plan file.
- Do not add a content status row or reduce editor viewport height.

### Task 1: Characterize Information Ownership

**Files:**
- Modify: `tests/ui_render.rs`

**Step 1: Add a wide-layout failing test**

Create an App with a named profile and active console target, set transaction to
MANUAL:ACTIVE, render at 120x36, and assert:

```rust
assert!(editor_top_border.contains("[profile] database.schema"));
assert!(editor_top_border.contains("TX MANUAL:ACTIVE"));
assert!(!header_lines.contains("TX MANUAL:ACTIVE"));
assert!(!output.contains(&profile.id.to_string()));
```

Use rendered row coordinates from `HitTarget::Focus(Focus::Editor)` rather than
searching the complete output for ownership-sensitive assertions.

**Step 2: Add state-table tests**

Cover AUTO, MANUAL:IDLE, STARTING, ACTIVE, ABORTED, COMMITTING, ROLLING BACK, and
OUTCOME UNKNOWN in the editor title.

**Step 3: Run and observe failures**

```bash
cargo test --test ui_render editor_context_title -- --nocapture
```

Expected: transaction and UUID target are in Header; editor right title is absent.

**Step 4: Commit red tests**

```bash
git add tests/ui_render.rs
git commit -m "test(ui): characterize editor context ownership"
```

### Task 2: Add the Editor Context Display Projection

**Files:**
- Modify: `src/ui/mod.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Define private projection types**

```rust
struct EditorContextDisplay {
    profile: Option<String>,
    database: Option<String>,
    schema: Option<String>,
    target_state: TargetDisplayState,
    transaction: &'static str,
}

enum TargetDisplayState {
    Ready,
    Linking,
    Offline,
    Missing,
    Invalid,
}
```

**Step 2: Build the projection from App state**

Resolve `ExecutionTarget.profile_id` to `ConnectionProfile.name`. Sanitize every
display component with existing terminal-safe projection. Determine target state
by comparing target profile with active and pending connection identities and by
calling target validation against the profile.

Never format profile UUID into a UI string.

**Step 3: Add projection unit tests if branching becomes non-trivial**

Cover missing profile, invalid schema, matching active connection, pending
connection, and offline target.

**Step 4: Run focused tests**

```bash
cargo test --test ui_render editor_context_title -- --nocapture
```

Expected: projection tests pass; render ownership tests remain red until Task 3.

**Step 5: Commit**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "refactor(ui): project editor context labels"
```

### Task 3: Render Native Left and Right Block Titles

**Files:**
- Modify: `src/ui/mod.rs:280-394,516-620`
- Modify: `tests/ui_render.rs`

**Step 1: Remove editor-local data from Header**

Delete target and transaction construction from `render_header`. Retain:

- app/profile identity;
- actual connected database;
- connection status;
- query status.

The Header must not display editor profile UUID, target database/schema, or
transaction.

**Step 2: Build one editor block with two top titles**

Replace separate `panel_block` calls with one block:

```rust
let block = panel_block("", focused, theme)
    .title_top(Line::from(left).left_aligned())
    .title_top(Line::from(right).right_aligned());
let inner = block.inner(area);
frame.render_widget(block, area);
```

Do not render a second block later; calculate `inner` from the same block.

**Step 3: Style context semantics**

Use normal/action colors for Ready, warning for Linking/Offline, and error for
Missing/Invalid or dangerous transaction states. Every state also has text.

**Step 4: Run wide UI tests**

```bash
cargo test --test ui_render editor_context_title -- --nocapture
```

Expected: PASS for ownership, names, transaction states, and no UUID.

**Step 5: Run completion popup positioning tests**

```bash
cargo test --test ui_render completion_popup -- --nocapture
```

Expected: PASS because inner editor geometry is unchanged.

**Step 6: Commit**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(ui): render editor target and transaction title"
```

### Task 4: Implement Responsive Title Degradation

**Files:**
- Modify: `src/ui/mod.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Add width-specific failing tests**

Render standard/compact layouts and inspect the editor top border:

- 120x36: profile + database.schema + full transaction;
- 80x24: database.schema + full transaction;
- 56x16: schema or target state + full transaction;
- smaller editor width: transaction only;
- extreme width: short transaction label.

**Step 2: Add a pure width-selection function**

```rust
fn editor_context_title(display: &EditorContextDisplay, available: usize) -> String
```

Prepare candidates from most detailed to least detailed and choose the first that
fits. Transaction is present in every candidate. Use terminal cell width, not
`.len()`.

**Step 3: Protect left and right title collision**

Compute right-title budget after reserving the left title and border spacing. If
only the transaction candidate fits, use it. Do not allow titles to overwrite
editor text or each other.

**Step 4: Run width tests**

```bash
cargo test --test ui_render editor_context_title -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "fix(ui): prioritize transaction in narrow editor titles"
```

### Task 5: Expand Contextual Help

**Files:**
- Modify: `src/ui/mod.rs:1179-1234`
- Modify: `tests/ui_render.rs`
- Modify: `docs/keybindings.md`

**Step 1: Add failing Help assertions**

At 80x24, assert Editor Help contains:

```text
Space d
:connection
:database
:schema
Space tt
Space tc
Space tr
:commit
:rollback
```

Also assert Help explains that title target mismatch blocks execution and MANUAL
restores as Idle.

**Step 2: Group and shorten Help lines**

Add Execution Target and Transaction headings. Increase popup maximum height only
within terminal bounds. Keep critical lines visible at 80x24; avoid adding a new
scroll state for this change.

**Step 3: Update keybinding documentation**

Document title format and target/transaction controls after render tests pass.

**Step 4: Run Help and complete UI tests**

```bash
cargo test --test ui_render help_overlay -- --nocapture
cargo test --test ui_render
```

Expected: PASS.

**Step 5: Commit**

```bash
git add src/ui/mod.rs tests/ui_render.rs docs/keybindings.md
git commit -m "docs(ui): explain editor target and transaction context"
```

### Task 6: Complete Regression Gates

**Step 1: Run complete gates**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

**Step 2: Manually inspect at representative widths**

Verify 120x36, 80x24, and compact layout. Check that completion popup anchoring,
cursor visibility, and SQL viewport dimensions did not change.

## Acceptance Checklist

- Header has no transaction or editor target.
- SQL Editor right title shows user-facing profile/database/schema and transaction.
- No profile UUID is visible.
- Transaction survives every responsive degradation tier.
- Help documents title semantics and all target/transaction controls.
- SQL text viewport and completion popup geometry are unchanged.
- Full gates pass.
