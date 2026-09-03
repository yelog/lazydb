# Focused Pane Maximize Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `Ctrl-w f` to toggle a maximized view of the currently focused pane without losing the user's pane-size preferences.

**Architecture:** Store one transient `pane_maximized` boolean in `App` and toggle it through a semantic `Action::TogglePaneMaximized`. Pass that state into `AppLayout::calculate` so an explicitly maximized wide terminal reuses the existing narrow-terminal `LayoutMode::Focus` behavior; keep header, footer, workspace tabs, result tabs, overlays, and notifications unchanged. Route the shortcut through both the application keymap and the editor's independent Vim window-command path, and do not persist the state or encode it by changing `PaneSizePreferences`.

**Tech Stack:** Rust 2024, Ratatui, Crossterm, ModalKit, Cargo tests

---

### Task 1: Add the Transient Application State and Action

**Files:**
- Modify: `src/action.rs:85-94`
- Modify: `src/app.rs:201-232`
- Modify: `src/app.rs:540-573`
- Modify: `src/app.rs:2552-2605`
- Test: `src/app.rs` test module near the existing pane resize test around line 13809

**Step 1: Write the failing reducer test**

Add a test next to the pane resize/reset reducer test. It must prove that maximization toggles in both directions and does not overwrite pane-size preferences:

```rust
#[test]
fn pane_maximize_toggles_without_changing_size_preferences() {
    let mut app = App::new(Vec::new());
    app.pane_sizes = PaneSizePreferences {
        explorer_width: Some(45),
        editor_height: Some(12),
    };
    let sizes = app.pane_sizes;

    assert!(!app.pane_maximized);
    assert!(app.update(Action::TogglePaneMaximized).is_empty());
    assert!(app.pane_maximized);
    assert_eq!(app.pane_sizes, sizes);

    assert!(app.update(Action::TogglePaneMaximized).is_empty());
    assert!(!app.pane_maximized);
    assert_eq!(app.pane_sizes, sizes);
}
```

**Step 2: Run the test to verify it fails**

Run: `cargo test pane_maximize_toggles_without_changing_size_preferences --lib`

Expected: FAIL because `App::pane_maximized` and `Action::TogglePaneMaximized` do not exist.

**Step 3: Add the minimal state and action**

Add the action alongside the existing focus and pane actions in `src/action.rs`:

```rust
Focus(Focus),
TogglePaneMaximized,
ResizePane(PaneResize),
```

Add the transient state alongside `focus` and `pane_sizes` in `App`:

```rust
pub focus: Focus,
pub pane_maximized: bool,
pub pane_sizes: PaneSizePreferences,
```

Initialize it in `App::with_profiles`:

```rust
focus: Focus::Editor,
pane_maximized: false,
pane_sizes: PaneSizePreferences::default(),
```

Handle it in `App::update` immediately after the focus actions and before pane resizing:

```rust
Action::TogglePaneMaximized => {
    self.pane_maximized = !self.pane_maximized;
    Vec::new()
}
```

Do not add this field to `WorkspaceSnapshot`, `PersistedProfileWorkspace`, settings, or any other persistence model.

**Step 4: Run the focused test**

Run: `cargo test pane_maximize_toggles_without_changing_size_preferences --lib`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/action.rs src/app.rs
git commit -m "feat(ui): add pane maximize state"
```

### Task 2: Map `Ctrl-w f` Outside the Editor

**Files:**
- Modify: `src/input/keymap.rs:1334-1393`
- Test: `tests/keymap.rs:847-876`

**Step 1: Write the failing keymap test**

Extend the window-command test or add a focused test using the existing `window_action` helper:

```rust
#[test]
fn maps_window_f_to_toggle_focused_pane_maximize() {
    let mut app = App::new(Vec::new());

    for focus in [Focus::Explorer, Focus::Results] {
        app.focus = focus;
        assert_eq!(
            window_action(&app, 'f'),
            Some(Action::TogglePaneMaximized)
        );
    }
}
```

The Explorer and Results cases cover the application-level keymap. Editor Normal mode is deliberately deferred to Task 3 because it has a separate ModalKit path.

**Step 2: Run the test to verify it fails**

Run: `cargo test --test keymap maps_window_f_to_toggle_focused_pane_maximize`

Expected: FAIL because the second key currently produces no action.

**Step 3: Add the window-prefix mapping**

In `map_pending`, add a count-independent mapping near `Ctrl-w =`:

```rust
(Pending::Window { .. }, KeyCode::Char('f')) => Some(Action::TogglePaneMaximized),
```

Do not add a new `Pending` variant. A prefix count such as `5 Ctrl-w f` should intentionally have the same toggle behavior as `Ctrl-w f`, just as `Ctrl-w =` ignores its count.

**Step 4: Run the focused keymap tests**

Run: `cargo test --test keymap maps_window_f_to_toggle_focused_pane_maximize`

Run: `cargo test --test keymap maps_counted_pane_resize_commands`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/input/keymap.rs tests/keymap.rs
git commit -m "feat(keymap): map ctrl-w f to pane maximize"
```

### Task 3: Route `Ctrl-w f` Through the Editor Vim Path

**Files:**
- Modify: `src/editor/mod.rs:40-55`
- Modify: `src/editor/mod.rs:1292-1331`
- Modify: `src/app.rs:7960-7980`
- Test: `src/editor/tests.rs:112-142`

**Step 1: Write the failing editor-effect test**

Add a test next to the existing editor window resize/reset tests:

```rust
#[test]
fn normal_mode_window_f_emits_toggle_maximize_effect() {
    let (mut workspace, id) = fixture("alpha");
    workspace.press(id, EditorKey::Escape).unwrap();
    workspace.press(id, EditorKey::Control('w')).unwrap();
    workspace.press(id, EditorKey::Character('f')).unwrap();

    assert_eq!(
        workspace.drain_effects(),
        vec![EditorEffect::TogglePaneMaximized]
    );
}
```

Keep the existing `insert_control_keys_keep_vim_semantics` test unchanged. It guards that Insert mode `Ctrl-w` still deletes the previous word rather than starting a pane command.

**Step 2: Run the test to verify it fails**

Run: `cargo test normal_mode_window_f_emits_toggle_maximize_effect --lib`

Expected: FAIL because `EditorEffect::TogglePaneMaximized` does not exist.

**Step 3: Add and emit the editor effect**

Add a variant beside the existing pane effects:

```rust
ResizePane(PaneResize),
ResetPaneSizes,
TogglePaneMaximized,
```

Handle the editor's pending window binding in `input_vim_key`:

```rust
(PendingBinding::Window(_), 'f') => {
    self.effects.push(EditorEffect::TogglePaneMaximized)
}
```

Map the drained effect back to the application action in `App::apply_editor_effects`:

```rust
EditorEffect::TogglePaneMaximized => Action::TogglePaneMaximized,
```

**Step 4: Run editor and application tests**

Run: `cargo test normal_mode_window_f_emits_toggle_maximize_effect --lib`

Run: `cargo test insert_control_keys_keep_vim_semantics --lib`

Run: `cargo test normal_mode_counted_window_resize_emits_shared_effect --lib`

Expected: PASS. The Insert mode assertion must continue to prove `Ctrl-w` deletes a word.

**Step 5: Commit**

```bash
git add src/editor/mod.rs src/editor/tests.rs src/app.rs
git commit -m "feat(editor): support pane maximize window command"
```

### Task 4: Reuse Focus Layout for Explicit Maximization

**Files:**
- Modify: `src/ui/layout.rs:36-186`
- Modify: `src/ui/layout.rs:219-341`
- Modify: `src/ui/mod.rs:525-535`

**Step 1: Update layout tests to pass the new argument**

Add `false` as the final argument to every existing `AppLayout::calculate` call in `src/ui/layout.rs` tests. This preserves all existing behavior while making the new state explicit.

Example:

```rust
let layout = AppLayout::calculate(
    Rect::new(0, 0, 180, 50),
    Focus::Editor,
    false,
    PaneSizePreferences::default(),
    false,
);
```

**Step 2: Add failing maximized-layout tests**

Add one SQL-layout test that loops through all three focus values:

```rust
#[test]
fn maximized_sql_layout_only_exposes_the_focused_pane() {
    for focus in [Focus::Explorer, Focus::Editor, Focus::Results] {
        let layout = AppLayout::calculate(
            Rect::new(0, 0, 180, 50),
            focus,
            false,
            PaneSizePreferences::default(),
            true,
        );

        assert_eq!(layout.mode, LayoutMode::Focus);
        assert_eq!(layout.explorer.is_some(), focus == Focus::Explorer);
        assert_eq!(layout.editor.is_some(), focus == Focus::Editor);
        assert_eq!(layout.results.is_some(), focus == Focus::Results);
        assert_eq!(layout.pane_metrics, PaneLayoutMetrics::default());
    }
}
```

Add relation/dashboard-layout coverage, where `is_relation` is the shared two-pane layout flag:

```rust
#[test]
fn maximized_relation_layout_uses_results_as_the_main_pane() {
    let layout = AppLayout::calculate(
        Rect::new(0, 0, 180, 50),
        Focus::Results,
        true,
        PaneSizePreferences::default(),
        true,
    );

    assert_eq!(layout.mode, LayoutMode::Focus);
    assert!(layout.explorer.is_none());
    assert!(layout.editor.is_none());
    assert!(layout.results.is_none());
    assert!(layout.relation.is_some());
    assert!(layout.tabs.is_some());
    assert_eq!(layout.pane_metrics, PaneLayoutMetrics::default());
}
```

**Step 3: Run the layout tests to verify they fail**

Run: `cargo test ui::layout::tests --lib`

Expected: FAIL because `AppLayout::calculate` does not yet accept explicit maximization and wide layouts remain multi-pane.

**Step 4: Extend the layout API with the minimal branch change**

Add a final parameter:

```rust
pub fn calculate(
    area: Rect,
    focus: Focus,
    is_relation: bool,
    preferences: PaneSizePreferences,
    pane_maximized: bool,
) -> Self
```

Change only the existing focus-layout condition:

```rust
if pane_maximized || area.width < 100 {
```

Do not duplicate the Focus layout or construct rectangles in `src/ui/mod.rs`. Keep `TooSmall` higher priority so terminals below the existing `56x16` minimum continue to render the current warning.

Pass the application state from the root renderer:

```rust
let layout = AppLayout::calculate(
    area,
    app.focus,
    is_relation || is_dashboard,
    app.pane_sizes,
    app.pane_maximized,
);
```

The existing Focus branch intentionally preserves header/footer and required tabs. Its default `PaneLayoutMetrics` also prevents maximized dimensions from replacing the remembered split metrics.

**Step 5: Run layout tests**

Run: `cargo test ui::layout::tests --lib`

Expected: PASS, including all pre-existing narrow, standard, wide, and pane preference tests.

**Step 6: Commit**

```bash
git add src/ui/layout.rs src/ui/mod.rs
git commit -m "feat(ui): render focused pane in maximize mode"
```

### Task 5: Add the Shortcut to Contextual Help

**Files:**
- Modify: `src/help.rs:233-250`
- Modify: `src/help.rs:705-780`
- Modify: `src/help.rs:3954-4058`
- Modify: `src/app.rs:1380-1610`
- Test: `src/help.rs` test module
- Test: `tests/keymap.rs`

**Step 1: Add failing help catalog assertions**

Update the exact expected `ShortcutPrefix::Window` candidate lists in `prefix_candidates_match_leader_and_window_availability` to include the new ID after the directional focus commands and before resize commands:

```rust
HelpShortcutId::TogglePaneMaximized,
```

Add explicit context coverage:

```rust
#[test]
fn pane_maximize_help_is_available_only_in_pane_navigation_contexts() {
    for context in [
        ShortcutContext::Explorer,
        ShortcutContext::EditorNormal,
        ShortcutContext::EditorVisual,
        ShortcutContext::SqlResultsData,
        ShortcutContext::SqlOutput,
        ShortcutContext::Dashboard,
        ShortcutContext::RelationDataBrowse,
        ShortcutContext::RelationDataVisual,
        ShortcutContext::RelationDdl,
    ] {
        assert!(
            prefix_ids(
                context,
                ShortcutCapabilities::default(),
                ShortcutPrefix::Window,
            )
            .contains(&HelpShortcutId::TogglePaneMaximized)
        );
    }

    for context in [
        ShortcutContext::EditorInsert,
        ShortcutContext::RelationDataEdit,
        ShortcutContext::DataQueryInput,
        ShortcutContext::ProfileManagerForm,
    ] {
        assert!(!shortcuts(context, ShortcutCapabilities::default())
            .iter()
            .any(|shortcut| shortcut.id == HelpShortcutId::TogglePaneMaximized));
    }
}
```

If `Shortcut` fields are private to the module test, the direct ID comparison above is valid because these tests live in `src/help.rs`. Use the existing `prefix_ids` and `shortcuts` helpers rather than adding production-only accessors.

**Step 2: Run the help tests to verify they fail**

Run: `cargo test help::tests::prefix_candidates_match_leader_and_window_availability --lib`

Run: `cargo test pane_maximize_help_is_available_only_in_pane_navigation_contexts --lib`

Expected: FAIL because the shortcut ID and catalog row do not exist.

**Step 3: Add the help ID and catalog row**

Add:

```rust
TogglePaneMaximized,
```

to `HelpShortcutId` near the focus and pane-size IDs.

Add one catalog row after the directional focus rows:

```rust
row!(
    TogglePaneMaximized,
    [
        Explorer,
        EditorNormal,
        EditorVisual,
        SqlResultsData,
        SqlOutput,
        Dashboard,
        RelationDataBrowse,
        RelationDataVisual,
        RelationDdl
    ],
    "Ctrl-w f",
    "maximize or restore focused pane",
    Window,
    "f"
),
```

Use the generic Window-prefix macro arm because this action is always available in the listed pane contexts and has no direction or resize capability requirement.

Map execution from the help overlay in `App::execute_help_shortcut`:

```rust
Id::TogglePaneMaximized => vec![Action::TogglePaneMaximized],
```

**Step 4: Add help execution integration coverage**

In `tests/keymap.rs`, add a test following the existing executable-help tests:

```rust
#[test]
fn pane_maximize_help_entry_executes_the_same_action() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;
    app.update(Action::ShowHelp);
    app.update(Action::HelpPaste("maximize or restore focused pane".into()));

    assert_eq!(
        app.help_selected_id(),
        Some(lazydb::help::HelpShortcutId::TogglePaneMaximized)
    );
    app.update(Action::ExecuteHelpShortcut(
        lazydb::help::HelpShortcutId::TogglePaneMaximized,
    ));

    assert!(app.pane_maximized);
    assert_eq!(app.overlay, None);
}
```

**Step 5: Run help and keymap tests**

Run: `cargo test help::tests --lib`

Run: `cargo test --test keymap pane_maximize`

Expected: PASS. Existing catalog uniqueness, prefix reconstruction, context coverage, footer ordering, and executable shortcut tests must also remain green.

**Step 6: Commit**

```bash
git add src/help.rs src/app.rs tests/keymap.rs
git commit -m "feat(help): document focused pane maximize"
```

### Task 6: Verify Root Rendering and Regression Boundaries

**Files:**
- Test: `tests/ui_render.rs`

**Step 1: Add a root-render integration test**

Use the existing `fixture`, `TestBackend`, `UiState`, and hit-region machinery. Render at a wide size so the test distinguishes explicit maximization from responsive narrow mode:

```rust
#[test]
fn pane_maximize_hides_other_pane_hit_targets_and_restores_them() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    let mut state = UiState::new();

    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(state.hit_regions.iter().any(
        |region| region.target == HitTarget::Focus(Focus::Explorer)
    ));
    assert!(state.hit_regions.iter().any(
        |region| region.target == HitTarget::Focus(Focus::Editor)
    ));
    assert!(state.hit_regions.iter().any(
        |region| region.target == HitTarget::Focus(Focus::Results)
    ));

    app.update(Action::TogglePaneMaximized);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(!state.hit_regions.iter().any(
        |region| region.target == HitTarget::Focus(Focus::Explorer)
    ));
    assert!(state.hit_regions.iter().any(
        |region| region.target == HitTarget::Focus(Focus::Editor)
    ));
    assert!(!state.hit_regions.iter().any(
        |region| region.target == HitTarget::Focus(Focus::Results)
    ));

    app.update(Action::TogglePaneMaximized);
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    assert!(state.hit_regions.iter().any(
        |region| region.target == HitTarget::Focus(Focus::Explorer)
    ));
    assert!(state.hit_regions.iter().any(
        |region| region.target == HitTarget::Focus(Focus::Results)
    ));
}
```

If `UiState::new` is not public to integration tests, follow the file's existing constructor pattern. If `HitTarget` equality is inconvenient, use `matches!(&region.target, HitTarget::Focus(...))`; do not add production API solely for this assertion.

**Step 2: Run the test**

Run: `cargo test --test ui_render pane_maximize_hides_other_pane_hit_targets_and_restores_them`

Expected: PASS. If it fails, fix only the root layout state propagation or stale hit-region clearing; do not special-case individual renderers.

**Step 3: Run all focused feature tests**

Run: `cargo test pane_maximize --lib`

Run: `cargo test --test keymap pane_maximize`

Run: `cargo test --test ui_render pane_maximize`

Run: `cargo test ui::layout::tests --lib`

Expected: PASS.

**Step 4: Run formatting and static checks**

Run: `cargo fmt --check`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS with no formatting diff and no warnings.

If formatting fails, run `cargo fmt`, inspect the resulting diff, and repeat `cargo fmt --check`.

**Step 5: Run the complete test suite**

Run: `cargo test`

Expected: PASS.

Pay particular attention to regressions in:

- Existing narrow-terminal Focus layout behavior.
- Editor Insert mode `Ctrl-w` word deletion.
- Counted pane resize commands.
- Help prefix candidate ordering and catalog completeness.
- Relation and Dashboard two-pane rendering.
- Workspace persistence tests, which should show no schema/version changes.

**Step 6: Inspect the final diff**

Run: `git diff --check`

Run: `git status --short`

Run: `git diff -- src/action.rs src/app.rs src/input/keymap.rs src/editor/mod.rs src/editor/tests.rs src/help.rs src/ui/mod.rs src/ui/layout.rs tests/keymap.rs tests/ui_render.rs`

Expected: only the focused-pane maximize implementation and its tests are present; no persistence file or workspace version is modified.

**Step 7: Commit**

```bash
git add tests/ui_render.rs
git commit -m "test(ui): cover focused pane maximize rendering"
```

### Acceptance Criteria

- `Ctrl-w f` maximizes the currently focused Explorer, Editor, Results, Relation, or Dashboard pane in supported navigation contexts.
- Pressing `Ctrl-w f` again restores the normal multi-pane layout.
- Header, footer, workspace tabs, result tabs, overlays, notifications, and key-sequence UI continue to render according to the existing Focus layout.
- Maximization does not modify `PaneSizePreferences`; restoring returns to the user's previous split sizes.
- Maximization is transient and is not added to workspace or settings persistence.
- Editor Normal/Visual window commands support `Ctrl-w f`; Editor Insert/Replace keeps its existing `Ctrl-w` text-editing behavior.
- Narrow terminals retain their existing automatic Focus layout.
- Application keymap, editor effects, contextual help execution, layout unit tests, root rendering tests, Clippy, and the full test suite pass.
