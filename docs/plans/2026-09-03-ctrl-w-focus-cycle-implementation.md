# Ctrl-W Focus Cycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Vim-compatible `Ctrl-w Ctrl-w` pane focus cycling, moving clockwise through every pane that exists in the active workspace layout.

**Architecture:** Reuse the existing `Action::FocusNext` reducer as the single source of truth for pane order and missing-pane handling. Extend both existing window-command input paths: application-level `Keymap::Pending::Window` for Explorer/Results and editor-owned `PendingBinding::Window` for SQL Editor Normal/Visual modes. Preserve Insert/Replace `Ctrl-w` as delete-previous-word, and expose the new continuation through the shared help catalog and keyboard contract.

**Tech Stack:** Rust 2024, Crossterm key events, ModalKit-backed editor input, Ratatui contextual help, Cargo tests

---

### Task 1: Map `Ctrl-w Ctrl-w` Outside the SQL Editor

**Files:**
- Modify: `src/input/keymap.rs:1378-1439`
- Test: `tests/keymap.rs:644-696`

**Step 1: Write the failing application-keymap test**

Add a focused test next to the existing `window_action` tests. Use actual control-modified events for both keys rather than the `window_action` helper, whose continuation is an unmodified character:

```rust
#[test]
fn ctrl_w_ctrl_w_maps_to_focus_next_outside_editor() {
    let mut app = App::new(Vec::new());

    for focus in [Focus::Explorer, Focus::Results] {
        app.focus = focus;
        let mut keymap = Keymap::default();

        assert_eq!(keymap.map(ctrl('w'), &app), None);
        assert_eq!(keymap.map(ctrl('w'), &app), Some(Action::FocusNext));
    }
}
```

Add a modifier regression test proving that plain `w` is not accepted as the cycle continuation:

```rust
#[test]
fn ctrl_w_plain_w_does_not_cycle_focus() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(keymap.map(key(KeyCode::Char('w')), &app), None);
}
```

**Step 2: Run the focused tests to verify they fail**

Run: `cargo test --test keymap ctrl_w_ctrl_w_maps_to_focus_next_outside_editor`

Expected: FAIL because the second control-modified `w` currently has no `Pending::Window` mapping.

Run: `cargo test --test keymap ctrl_w_plain_w_does_not_cycle_focus`

Expected: PASS before and after implementation; this is a guard against accidentally matching only `KeyCode::Char('w')`.

**Step 3: Add the minimal pending-window mapping**

In `map_pending`, add this arm before the directional `h/j/k/l` arms:

```rust
(Pending::Window { .. }, KeyCode::Char('w'))
    if event.modifiers == KeyModifiers::CONTROL =>
{
    Some(Action::FocusNext)
}
```

Do not add a new `Action`, a new `Pending` variant, or a second focus-order function. `Action::FocusNext` already implements SQL's three-pane cycle and the two-pane Relation/Dashboard cycle.

Do not map unmodified `w`. `map_pending` deliberately permits modifiers for all window commands, so the arm itself must enforce `KeyModifiers::CONTROL`.

Counts are intentionally ignored for this continuation, matching Vim's effective behavior for repeated window cycling and matching the existing count-independent `Ctrl-w =` and `Ctrl-w f` actions.

**Step 4: Run the focused and window-command regression tests**

Run: `cargo test --test keymap ctrl_w_ctrl_w_maps_to_focus_next_outside_editor`

Run: `cargo test --test keymap ctrl_w_plain_w_does_not_cycle_focus`

Run: `cargo test --test keymap window_`

Expected: PASS. Existing direction, resize, reset, maximize, count, and pending-sequence behavior must remain unchanged.

**Step 5: Commit**

```bash
git add src/input/keymap.rs tests/keymap.rs
git commit -m "feat(keymap): cycle pane focus with ctrl-w ctrl-w"
```

### Task 2: Route the Sequence Through the Editor Vim Path

**Files:**
- Modify: `src/editor/mod.rs:40-56`
- Modify: `src/editor/mod.rs:911-1044`
- Modify: `src/app.rs:7938-8015`
- Test: `src/editor/tests.rs:102-155`

**Step 1: Write failing editor-effect tests**

Add a Normal-mode test beside the existing editor window-command tests:

```rust
#[test]
fn normal_mode_ctrl_w_ctrl_w_emits_focus_next_effect() {
    let (mut workspace, id) = fixture("alpha");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();

    assert_eq!(workspace.drain_effects(), vec![EditorEffect::FocusNext]);
}
```

Add one Visual-mode case using the editor test suite's existing helper or key sequence for entering Visual mode:

```rust
#[test]
fn visual_mode_ctrl_w_ctrl_w_emits_focus_next_effect() {
    let (mut workspace, id) = fixture("alpha");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Character('v')).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();

    assert_eq!(workspace.drain_effects(), vec![EditorEffect::FocusNext]);
}
```

Keep `insert_control_keys_keep_vim_semantics` unchanged. It is the regression contract that Insert-mode `Ctrl-w` deletes the previous word.

**Step 2: Run the editor tests to verify they fail**

Run: `cargo test normal_mode_ctrl_w_ctrl_w_emits_focus_next_effect --lib`

Expected: FAIL because the second `Ctrl-w` currently resets `PendingBinding::Window(1)` instead of completing it.

Run: `cargo test visual_mode_ctrl_w_ctrl_w_emits_focus_next_effect --lib`

Expected: FAIL because Visual mode does not currently establish the editor-owned window pending binding for a control-modified continuation.

**Step 3: Add a semantic editor effect**

Add the effect beside `FocusPane(Focus)` in `EditorEffect`:

```rust
FocusPane(Focus),
FocusNext,
```

In `App::apply_editor_effects`, map it to the existing reducer action:

```rust
EditorEffect::FocusPane(focus) => Action::Focus(focus),
EditorEffect::FocusNext => Action::FocusNext,
```

Do not emit `FocusPane(Focus::Results)` directly. The editor must describe the semantic operation, while `App::update(Action::FocusNext)` remains responsible for layout-aware ordering.

**Step 4: Complete an existing editor window binding with control-modified `w`**

In `EditorWorkspace::press`, after reading the current mode and before creating a new window pending binding, detect whether the session already has `PendingBinding::Window(_)` and the new key is `EditorKey::Control('w')`:

```rust
if key == EditorKey::Control('w')
    && self.sessions.get(&id).is_some_and(|session| {
        matches!(session.pending_binding, Some(PendingBinding::Window(_)))
    })
{
    self.sessions
        .get_mut(&id)
        .ok_or(EditorError::MissingSession(id))?
        .pending_binding = None;
    self.effects.push(EditorEffect::FocusNext);
    return Ok(());
}
```

Then allow Normal and Visual modes to establish the first window prefix:

```rust
(
    EditorMode::Normal
    | EditorMode::VisualChar
    | EditorMode::VisualLine
    | EditorMode::VisualBlock,
    EditorKey::Control('w'),
) => {
    self.sessions
        .get_mut(&id)
        .ok_or(EditorError::MissingSession(id))?
        .pending_binding = Some(PendingBinding::Window(1));
    Ok(())
}
```

Retain the later Insert/Replace arm exactly as a text-edit operation:

```rust
(EditorMode::Insert | EditorMode::Replace, EditorKey::Control('w')) => {
    self.delete_previous_word(id)
}
```

Do not move editor Normal/Visual handling into the application `Keymap`; editor counts and existing `Ctrl-w h/j/+/-/>/</=/f` behavior already live in `PendingBinding::Window`.

**Step 5: Run focused editor tests and mode regressions**

Run: `cargo test ctrl_w_ctrl_w_emits_focus_next_effect --lib`

Run: `cargo test insert_control_keys_keep_vim_semantics --lib`

Run: `cargo test normal_mode_counted_window_resize_emits_shared_effect --lib`

Run: `cargo test normal_mode_window_ --lib`

Expected: PASS. Normal and Visual cycle effects are emitted; Insert/Replace deletion, counted resize, reset, and maximize remain intact.

**Step 6: Commit**

```bash
git add src/editor/mod.rs src/editor/tests.rs src/app.rs
git commit -m "feat(editor): support ctrl-w focus cycling"
```

### Task 3: Verify Clockwise Cycling Across Workspace Layouts

**Files:**
- Test: `tests/keymap.rs` near the window-command integration tests
- Reference only: `src/model/workspace.rs:26-50`
- Reference only: `src/app.rs:2555-2575`

**Step 1: Add a small runtime-like key application helper in the test module**

The production runtime maps each key and immediately applies any returned action. Mirror that behavior locally:

```rust
fn apply_key(keymap: &mut Keymap, app: &mut App, event: KeyEvent) {
    if let Some(action) = keymap.map(event, app) {
        app.update(action);
    }
}

fn cycle_focus(keymap: &mut Keymap, app: &mut App) {
    apply_key(keymap, app, ctrl('w'));
    apply_key(keymap, app, ctrl('w'));
}
```

If equivalent helpers already exist by implementation time, reuse them rather than adding duplicates.

**Step 2: Write the failing SQL three-pane integration test**

```rust
#[test]
fn ctrl_w_ctrl_w_cycles_sql_panes_clockwise() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Esc)));
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Editor);
    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Results);
    cycle_focus(&mut keymap, &mut app);
    assert_eq!(app.focus, Focus::Explorer);
}
```

This test is important because its second transition exercises the editor-owned `EditorEffect::FocusNext` path, while the first and third exercise application-level `Pending::Window`.

**Step 3: Write two-pane Relation and Dashboard integration tests**

Add a Relation test asserting:

```text
Explorer -> Results -> Explorer
```

Construct the Relation tab using the same fixture pattern as `relation_window_directions_target_existing_panes`.

Add a Dashboard test asserting:

```text
Explorer -> Results -> Explorer
```

Construct the Dashboard tab using the same fixture pattern as `dashboard_cycles_focus_only_between_explorer_and_results` in `tests/workspace_tabs.rs`.

These tests prove `Action::FocusNext` skips the absent Editor pane rather than producing an invalid intermediate focus.

**Step 4: Run the integration tests**

Run: `cargo test --test keymap ctrl_w_ctrl_w_cycles_`

Expected: PASS for SQL, Relation, and Dashboard layouts.

**Step 5: Run the existing reducer focus tests**

Run: `cargo test cycles_focus_in_both_directions --lib`

Run: `cargo test --test relation_tabs relation_focus_cycles_only_explorer_and_results`

Run: `cargo test --test workspace_tabs dashboard_cycles_focus_only_between_explorer_and_results`

Expected: PASS. No focus-order or normalization behavior changes are required.

**Step 6: Commit**

```bash
git add tests/keymap.rs
git commit -m "test(keymap): cover pane focus cycles across layouts"
```

### Task 4: Add Contextual Help and Keyboard Documentation

**Files:**
- Modify: `src/help.rs:234-249`
- Modify: `src/help.rs:713-804`
- Modify: `src/help.rs:2498-2543`
- Modify: `src/app.rs:1382-1409`
- Modify: `docs/keybindings.md:31-75`
- Test: `src/help.rs` test module near `sql_window_directions_match_three_pane_mapping`
- Test: `tests/keymap.rs` near the existing executable-help tests

**Step 1: Write failing help-catalog tests**

Add a catalog test proving the shortcut is available in every non-input workspace context and is listed as a `Window` continuation:

```rust
#[test]
fn window_prefix_lists_clockwise_focus_cycle() {
    for context in [
        ShortcutContext::Explorer,
        ShortcutContext::EditorNormal,
        ShortcutContext::EditorVisual,
        ShortcutContext::SqlResultsData,
        ShortcutContext::SqlOutput,
        ShortcutContext::RelationDataBrowse,
        ShortcutContext::RelationDataVisual,
        ShortcutContext::RelationDdl,
        ShortcutContext::Dashboard,
    ] {
        let rows = prefix_shortcuts(
            context,
            ShortcutCapabilities::default(),
            ShortcutPrefix::Window,
        );
        assert!(rows.iter().any(|row| {
            row.id == HelpShortcutId::CyclePaneFocus
                && row.sequence == "Ctrl-w Ctrl-w"
                && row.suffix == Some("Ctrl-w")
        }));
    }
}
```

Adjust `ShortcutCapabilities` per context where current availability checks require the corresponding focus or relation layout. Follow the fixture construction used by the adjacent SQL and Relation window-direction tests rather than weakening capability checks.

Add an executable-help integration test that opens Help, filters to the cycle row, executes it, and asserts that Help closes and focus advances.

**Step 2: Run tests to verify they fail**

Run: `cargo test window_prefix_lists_clockwise_focus_cycle --lib`

Expected: FAIL because `HelpShortcutId::CyclePaneFocus` and its catalog row do not exist.

Run the exact executable-help test name with: `cargo test --test keymap <test-name>`

Expected: FAIL for the same missing shortcut ID or missing execution mapping.

**Step 3: Add the shortcut ID, catalog row, and ordering**

Add a semantic ID near the existing focus IDs:

```rust
FocusEditorFromL,
CyclePaneFocus,
TogglePaneMaximized,
```

Add this row after directional focus rows and before pane maximize/resize rows:

```rust
row!(
    CyclePaneFocus,
    [
        Explorer,
        EditorNormal,
        EditorVisual,
        SqlResultsData,
        SqlOutput,
        RelationDataBrowse,
        RelationDataVisual,
        RelationDataBusy,
        RelationDdl,
        Dashboard
    ],
    "Ctrl-w Ctrl-w",
    "cycle pane focus clockwise",
    Window,
    "Ctrl-w",
    always
),
```

Do not include `EditorInsert`, `RelationDataEdit`, search inputs, forms, or overlays, because those contexts own `Ctrl-w` as text deletion or modal input.

Add it to the `Window` prefix ordering:

```rust
Id::CyclePaneFocus => 4,
Id::TogglePaneMaximized => 5,
```

Increment later ranks in that branch so directional focus remains first, cycle follows directions, and pane management follows cycle.

The existing `always` row macro accepts a string suffix, so use `"Ctrl-w"`; do not encode the continuation as plain `"w"` because that would misrepresent the required modifier.

**Step 4: Make the Help row executable**

In `App::execute_help_shortcut`, route the new semantic ID to the existing action:

```rust
Id::CyclePaneFocus => vec![Action::FocusNext],
```

Do not synthesize two `EditorKey` events. Help execution is a command invocation and should directly use the semantic reducer action.

**Step 5: Update the keyboard contract**

In `docs/keybindings.md`:

- Add `Ctrl-w Ctrl-w` to the Global or Pane Navigation table as “Cycle clockwise through panes in the active layout.”
- State the concrete SQL order: `Explorer -> Editor -> Results -> Explorer`.
- State that Relation and Dashboard cycle only between `Explorer` and `Results`.
- Add `Ctrl-w` to the `Ctrl-w` prefix continuation list.
- Preserve the explicit statement that Editor Insert/Replace `Ctrl-w` deletes the previous word and never starts a pane command.

Suggested pane-navigation text:

```markdown
`Ctrl-w Ctrl-w` cycles clockwise through panes that exist in the active layout.
SQL uses `Explorer -> Editor -> Results -> Explorer`; Relation and Dashboard
use `Explorer -> Results -> Explorer`.
```

**Step 6: Run help and documentation-related tests**

Run: `cargo test window_prefix_lists_clockwise_focus_cycle --lib`

Run: `cargo test window_directions --lib`

Run: `cargo test --test keymap sql_help_window_directions_match_three_pane_mapping`

Run: `cargo test --test keymap relation_help_window_directions_match_two_pane_mapping`

Run the new executable-help integration test by exact name.

Expected: PASS. Existing capability-filtered direction rows remain unchanged, and the cycle row appears consistently in every supported pane context.

**Step 7: Commit**

```bash
git add src/help.rs src/app.rs tests/keymap.rs docs/keybindings.md
git commit -m "docs(keymap): expose ctrl-w focus cycle"
```

### Task 5: Run Full Verification

**Files:**
- Verify only; no planned source changes

**Step 1: Format the changed Rust files**

Run: `cargo fmt --check`

Expected: PASS. If it fails, run `cargo fmt`, inspect only the resulting intended formatting changes, then rerun `cargo fmt --check`.

**Step 2: Run all library tests**

Run: `cargo test --lib`

Expected: PASS.

**Step 3: Run all keymap and workspace integration tests**

Run: `cargo test --test keymap`

Run: `cargo test --test relation_tabs`

Run: `cargo test --test workspace_tabs`

Expected: PASS.

**Step 4: Run static analysis**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS with no warnings.

**Step 5: Run the complete test suite**

Run: `cargo test --all-targets --all-features`

Expected: PASS.

**Step 6: Perform a manual TUI smoke test**

Run the application using the repository's normal local startup command and verify:

1. On a SQL tab, repeated `Ctrl-w Ctrl-w` follows `Explorer -> Editor -> Results -> Explorer`.
2. On a Relation tab, repeated `Ctrl-w Ctrl-w` follows `Explorer -> Results -> Explorer`.
3. On Dashboard, repeated `Ctrl-w Ctrl-w` follows `Explorer -> Results -> Explorer`.
4. SQL Editor Insert/Replace still deletes the previous word on a single `Ctrl-w`.
5. `Ctrl-w h/j/k/l`, resize, reset, and maximize still work.
6. Pressing the first `Ctrl-w` shows `Ctrl-w Ctrl-w` in the pending-sequence candidates.
7. Waiting past `keybindings.sequence_timeout_ms` prevents the second `Ctrl-w` from cycling.

**Step 7: Commit any verification-only formatting fix if needed**

Only if `cargo fmt` changed intended feature files after the prior commits:

```bash
git add src/input/keymap.rs src/editor/mod.rs src/editor/tests.rs src/help.rs src/app.rs tests/keymap.rs
git commit -m "style: format ctrl-w focus cycle changes"
```

Do not commit unrelated worktree changes.

## Acceptance Criteria

1. `Ctrl-w Ctrl-w` cycles clockwise across all panes present in the active layout.
2. SQL cycles `Explorer -> Editor -> Results -> Explorer`.
3. Relation and Dashboard cycle `Explorer -> Results -> Explorer` without entering `Focus::Editor`.
4. The SQL Editor path works in Normal and Visual modes.
5. Insert/Replace single `Ctrl-w` continues deleting the previous word.
6. Plain `Ctrl-w w` does not trigger cycling; both keys must carry Control.
7. Existing sequence timeout, cancellation, state invalidation, and prefix UI behavior are reused.
8. Contextual Help and `docs/keybindings.md` document the shortcut and its layout-dependent order.
9. Existing `Ctrl-w` direction, resize, reset, maximize, and numeric-count behaviors do not regress.
10. Formatting, Clippy, focused tests, and the full test suite pass.
