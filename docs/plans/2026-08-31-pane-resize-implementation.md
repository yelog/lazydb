# Pane Resize Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Vim-style counted pane resizing and dynamic-default reset to LazyDB's fixed Explorer/Editor/Results layout while preserving native Editor count commands.

**Architecture:** Keep the current fixed layout and model its two adjustable split boundaries explicitly. `App` owns optional session preferences and the latest effective geometry, `AppLayout` remains the single source of layout constraints, and both global and Editor key handlers emit the same reducer actions.

**Tech Stack:** Rust 2024, Crossterm key events, Ratatui layout constraints, modalkit Editor integration, built-in Rust tests.

---

### Task 1: Define Pane Resize Domain Types

**Files:**
- Modify: `src/model/workspace.rs`
- Modify: `src/action.rs:29-55`

**Step 1: Write focused unit tests for resize translation**

Add tests in `src/model/workspace.rs` for a small domain helper that translates
focused-pane operators into split changes. Cover Explorer `>/<`, Editor
`+/-/>/<`, Results `+/-/>/<`, and the Explorer height no-op.

Use explicit types rather than raw tuple conventions:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PaneSizePreferences {
    pub explorer_width: Option<u16>,
    pub editor_height: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PaneLayoutMetrics {
    pub explorer_width: Option<u16>,
    pub editor_height: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneSplit {
    ExplorerWidth,
    EditorHeight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneResize {
    pub split: PaneSplit,
    pub delta: i32,
}
```

The translation helper should accept `Focus`, the operator character, and a
validated count, then return `Option<PaneResize>`. For example, Editor `>` must
produce a negative Explorer-width delta because growing the right side shrinks
Explorer; Results `+` must produce a negative Editor-height delta because
growing Results shrinks Editor.

**Step 2: Run the tests and verify they fail**

Run: `cargo test model::workspace::tests --lib`

Expected: FAIL because the pane resize types and translation helper do not yet
exist.

**Step 3: Implement the domain types and translation helper**

Use checked conversion from the parsed unsigned count to `i32`. Return `None`
for count zero, unsupported focus/operator combinations, or unrepresentable
counts. Do not put terminal-size constraints in this helper.

Add application actions:

```rust
Action::ResizePane(PaneResize)
Action::ResetPaneSizes
Action::PaneLayoutChanged(PaneLayoutMetrics)
```

Keep resize intent and rendered metrics separate: resize mutates preferences;
metrics synchronization only records effective geometry.

**Step 4: Run the tests**

Run: `cargo test model::workspace::tests --lib`

Expected: PASS.

**Step 5: Checkpoint**

Review the diff for domain-only changes. If commits are requested during
execution, commit as `feat(layout): define pane resize state`.

### Task 2: Make Layout Calculation Preference-Aware

**Files:**
- Modify: `src/ui/layout.rs:1-216`

**Step 1: Add failing layout tests**

Extend the existing `src/ui/layout.rs` test module to cover:

- `PaneSizePreferences::default()` preserves the current 34-to-56 Explorer
  default and 46-percent Editor default.
- Explicit Explorer width is applied and may exceed 56.
- Explorer width clamps to minimum 34 and leaves at least 60 columns right.
- Explicit Editor height is applied while preserving minimum Editor and Results
  areas and the fixed two-row result tabs.
- Relation tabs expose Explorer metrics but no Editor-height metric.
- Focus and TooSmall modes expose neither adjustable split.
- The same explicit preference is re-clamped after the terminal shrinks.
- Reset preferences (`None`, `None`) recompute defaults for a new terminal size.

Change test calls to pass preferences explicitly:

```rust
AppLayout::calculate(area, focus, is_relation, PaneSizePreferences::default())
```

**Step 2: Run the layout tests and verify they fail**

Run: `cargo test ui::layout::tests --lib`

Expected: FAIL because `calculate` does not accept preferences and layout does
not expose effective split metrics.

**Step 3: Centralize layout bounds**

Add named constants in `src/ui/layout.rs` for minimum terminal dimensions,
focus-mode width, Explorer minimum, right-side minimum, tab heights, and minimum
Editor/Results content. Replace matching literals in `calculate` so default and
manual sizing use one constraint definition.

Do not change the existing mode thresholds or responsive default formulas.

**Step 4: Apply preferences and expose metrics**

Extend `AppLayout` with:

```rust
pub pane_metrics: PaneLayoutMetrics,
```

Resolve each preference from the current available area, clamp before passing
lengths into Ratatui, and derive metrics from the resulting `Rect`s rather than
assuming requested lengths were honored. In Focus, TooSmall, and relation
vertical layouts, use `None` for unavailable split metrics.

**Step 5: Run layout tests**

Run: `cargo test ui::layout::tests --lib`

Expected: PASS.

**Step 6: Checkpoint**

Review only `src/ui/layout.rs`. If commits are requested, commit as
`feat(layout): apply pane size preferences`.

### Task 3: Store Preferences And Effective Metrics In App

**Files:**
- Modify: `src/app.rs:137-162,252-316,1228-1644`
- Test: `src/app.rs` existing test module near focus tests

**Step 1: Add failing reducer tests**

Add tests that initialize metrics and dispatch resize actions directly. Verify:

- Explorer width changes from the actual metric, not from a stale preference.
- Editor height changes from the actual metric.
- A missing metric makes that split a no-op.
- Repeated resize at a boundary does not accumulate hidden delta.
- A later reverse resize starts from the last effective boundary value.
- `ResetPaneSizes` clears both preferences.
- `PaneLayoutChanged` updates metrics without changing preferences.

Use small helper constructors in the test module only; do not add production
builders solely for tests.

**Step 2: Run the reducer tests and verify they fail**

Run: `cargo test pane_resize --lib`

Expected: FAIL because `App` has no pane state or reducer branches.

**Step 3: Add App-owned pane state**

Add public read access needed by rendering and private mutation ownership:

```rust
pub pane_sizes: PaneSizePreferences,
pane_layout: PaneLayoutMetrics,
```

Initialize both with `Default`. Keep them outside `ConnectionWorkspace` and all
persistence snapshots.

**Step 4: Implement reducer branches**

For `ResizePane`, read the relevant value from `pane_layout`, apply
`saturating_add_signed` or equivalent checked arithmetic, and store the result
as the corresponding preference. The renderer remains responsible for final
terminal-dependent clamping; the next metrics synchronization normalizes the
effective value.

For `ResetPaneSizes`, assign `PaneSizePreferences::default()`. For
`PaneLayoutChanged`, replace only the latest metrics.

Ensure these UI actions are not rejected by the early active-console guard in
`App::update`.

**Step 5: Run reducer tests**

Run: `cargo test pane_resize --lib`

Expected: PASS.

**Step 6: Checkpoint**

If commits are requested, commit as `feat(app): reduce pane resize actions`.

### Task 4: Synchronize Rendered Pane Metrics

**Files:**
- Modify: `src/ui/mod.rs:106-159,210-239`
- Modify: `src/runtime.rs:2641-2705` and the render synchronization call site

**Step 1: Add a failing render-state test**

Add or extend a UI test that renders a Standard layout and asserts
`UiState::pane_layout` equals the actual Explorer width and Editor height.
Render a relation tab or narrow terminal and assert the unavailable vertical
metric is `None`.

**Step 2: Run the focused UI test**

Run: `cargo test pane_layout_metrics --lib`

Expected: FAIL because `UiState` does not expose pane metrics.

**Step 3: Pass preferences into layout and retain metrics**

Add `pane_layout: PaneLayoutMetrics` to `UiState`, initialize it with default,
call `AppLayout::calculate(..., app.pane_sizes)`, and assign the calculated
metrics every render before any early return.

**Step 4: Synchronize metrics through runtime**

Add `sync_pane_layout` beside `sync_editor_viewport`. Dispatch
`Action::PaneLayoutChanged` only when `app`'s current metric differs, matching
the editor viewport's change-detection pattern and avoiding a redraw/action
loop. Add a narrow App getter if runtime cannot compare the private field
without exposing mutation.

Invoke synchronization in the same post-render phase as the existing editor,
grid, record-view, Explorer, and DDL viewport synchronization.

**Step 5: Run focused UI/runtime tests**

Run: `cargo test pane_layout_metrics --lib`

Expected: PASS.

Run: `cargo test runtime --lib`

Expected: PASS.

**Step 6: Checkpoint**

If commits are requested, commit as `feat(ui): synchronize pane geometry`.

### Task 5: Add Global Counted Window Resize Commands

**Files:**
- Modify: `src/input/keymap.rs:16-30,45-49,366-387,542-583,651-718`
- Test: `tests/keymap.rs:19-207` and window-command tests

**Step 1: Replace numeric-view tests with failing window-count tests**

Remove assertions that `1`, `2`, and `3` dispatch `SetResultView` or
`SetRelationView`. Add tests for Explorer and Results that verify:

- `Ctrl-w` followed by each of `+`, `-`, `>`, `<`, `=` dispatches the expected
  resize/reset action.
- `12 Ctrl-w >` dispatches one resize with count 12 translated for the current
  focus.
- `0 Ctrl-w >` is a no-op.
- An overflow-length digit sequence cannot panic and is a no-op or capped at the
  documented safe maximum.
- A non-window key after digits clears the count and still receives normal
  contextual mapping.
- Focus, tab, mode, and timeout changes invalidate both count and window pending
  state.
- Existing `Ctrl-w h/j/k/l` behavior remains unchanged.

**Step 2: Run keymap tests and verify they fail**

Run: `cargo test --test keymap window`

Expected: FAIL because resize operators and numeric prefixes are not mapped.

**Step 3: Refactor pending state without broad keymap changes**

Replace the tuple with a named pending-sequence struct containing the current
kind, start time, focus, editor mode, tab ID, and optional checked window count.
Keep the 750 ms timeout and existing context invalidation behavior.

Track leading digits only outside Editor focus. Accumulate with checked
arithmetic. If the next key is not `Ctrl-w`, clear the count and continue mapping
that key normally rather than swallowing it. Start `Pending::Window` with the
captured count or default count 1.

**Step 4: Map resize operators and reset**

Extend `map_pending` so `Pending::Window` handles `+`, `-`, `>`, `<`, and `=` in
addition to `h/j/k/l`. Use the shared focus/operator translation helper. Reset
ignores a count and always emits `Action::ResetPaneSizes`.

Remove `1`/`2`/`3` branches from `map_results` and `map_relation`.

**Step 5: Run keymap tests**

Run: `cargo test --test keymap window`

Expected: PASS.

Run: `cargo test --test keymap`

Expected: PASS, including contextual precedence tests.

**Step 6: Checkpoint**

If commits are requested, commit as `feat(keymap): add counted pane resize commands`.

### Task 6: Preserve Vim Counts While Adding Editor Resize Effects

**Files:**
- Modify: `src/editor/mod.rs:39-69,164-205,878-960,1193-1259`
- Modify: `src/app.rs` EditorEffect mapping near `EditorEffect::FocusPane`
- Test: `src/editor/tests.rs`

**Step 1: Add failing Editor tests**

Add tests for:

- `5`, `Ctrl-w`, `>` emits the Editor-focused resize action/effect with count 5.
- `Ctrl-w =` emits reset.
- `5j` still moves five lines through modalkit.
- `10G` still preserves the complete native count.
- Leading `0` still performs Vim's line-start command immediately.
- Insert/Replace `Ctrl-w` still deletes the previous word.
- Existing Editor `Ctrl-w h` and `Ctrl-w j` focus effects still pass.

Expose effects only through the existing test helpers; do not make Editor
internals public.

**Step 2: Run Editor tests and verify they fail**

Run: `cargo test editor::tests --lib`

Expected: FAIL on counted pane resize cases while existing Vim cases show the
behavior that must remain stable.

**Step 3: Add a dedicated pending window count to Editor sessions**

Store a small checked numeric buffer in each `EditorSession`. In Normal mode:

- A leading `1` through `9` starts the buffer.
- Following digits extend it with checked arithmetic.
- A leading `0` with no active count follows the existing modalkit path.
- `Ctrl-w` consumes the buffer into `PendingBinding::Window` state.
- Any other key first flushes buffered digits directly through the existing
  low-level Vim input path, then processes the current key.

Use a private helper to flush digits without recursively re-entering the new
prefix interception. Clear this state when sessions close or context changes as
appropriate.

**Step 4: Emit shared application actions**

Add Editor effects for pane resize and reset. Window operator handling uses the
stored count (default 1) and the shared Editor-focus translation helper.

Map these effects in `App` to `Action::ResizePane` and
`Action::ResetPaneSizes`, alongside the existing `FocusPane` mapping. Do not
duplicate reducer logic in Editor handling.

**Step 5: Run Editor and App tests**

Run: `cargo test editor::tests --lib`

Expected: PASS.

Run: `cargo test pane_resize --lib`

Expected: PASS.

**Step 6: Checkpoint**

If commits are requested, commit as `feat(editor): support counted pane resizing`.

### Task 7: Update Help And Remove Numeric View Selection Contract

**Files:**
- Modify: `src/help.rs:3-86,159-180` and related help tests
- Modify: `src/app.rs:995-1020` help shortcut execution mapping
- Modify: `docs/keybindings.md:16-30,164-215`
- Modify: `tests/keymap.rs`

**Step 1: Add failing help tests**

Update help tests to expect entries for the four resize operators, reset, and
count syntax in each applicable pane context. Remove expectations for direct
Results Data/Output/Plan numeric selection.

**Step 2: Run help tests and verify they fail**

Run: `cargo test help --lib`

Expected: FAIL against the old shortcut catalog.

**Step 3: Update executable contextual help**

Add shortcut IDs and descriptions for pane resizing/reset. Map executable help
entries through the same resize translation and reducer actions where a fixed
count of one applies. Remove `ResultsData`, `ResultsSecondaryView`, and
`ResultsPlan` shortcut IDs and their `App` execution branches if they are used
only by numeric direct selection.

Document `N Ctrl-w +|-|>|<` as count syntax; contextual help does not need one
entry per possible count.

**Step 4: Update the operational keybinding document**

In `docs/keybindings.md`:

- Add all pane resize/reset keys to Global.
- State that `Ctrl-w =` restores responsive defaults and does not equalize.
- State that unsupported dimensions are no-ops.
- Remove `1/2/3` result-view rows.
- Retain `o`, relation `p`, and relation `D`.

**Step 5: Run help and keymap tests**

Run: `cargo test help --lib`

Expected: PASS.

Run: `cargo test --test keymap`

Expected: PASS.

**Step 6: Checkpoint**

If commits are requested, commit as `docs(keymap): document pane resize commands`.

### Task 8: Full Verification And Regression Review

**Files:**
- Verify all modified files

**Step 1: Format**

Run: `cargo fmt --all -- --check`

Expected: PASS. If it fails, run `cargo fmt --all`, then rerun the check.

**Step 2: Run targeted tests together**

Run: `cargo test pane_resize --lib && cargo test ui::layout::tests --lib && cargo test editor::tests --lib && cargo test --test keymap`

Expected: PASS.

**Step 3: Run the full suite**

Run: `cargo test --all-targets`

Expected: PASS.

**Step 4: Run Clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS with no warnings.

**Step 5: Review behavior manually**

Launch LazyDB in a terminal and verify Standard/Wide, narrow Focus mode, SQL
tabs, relation tabs, Editor Normal mode, and Editor Insert mode. Resize the
terminal after setting preferences, test both boundaries, reverse direction at
a boundary, and confirm `Ctrl-w =` tracks the new terminal's defaults.

**Step 6: Final checkpoint**

Inspect `git diff --check`, `git status --short`, and the complete diff. Confirm
that no workspace persistence schema changed and no unrelated files were
modified. If commits are requested, create the final logical commit only for
remaining verification fixes; do not amend earlier commits.
