# Searchable Help Palette Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Convert the static contextual help overlay into a searchable, keyboard-navigable shortcut palette whose independently listed shortcuts can be executed with Enter.

**Architecture:** Add a presentation-neutral help state and shortcut catalog, route help input through dedicated actions, and resolve stable shortcut IDs to existing app/editor behavior only after closing the overlay. Render filtering and scrolling from the same catalog so displayed rows and executable actions cannot diverge.

**Tech Stack:** Rust 2024, crossterm 0.29, ratatui 0.30, existing reducer/command runtime, built-in Rust test framework.

---

### Task 1: Add Help State And Structured Shortcut Catalog

**Files:**
- Create: `src/help.rs`
- Modify: `src/lib.rs`
- Modify: `src/model/workspace.rs:48-51`
- Test: `src/help.rs`

**Step 1: Write failing catalog and filtering tests**

Add unit tests that assert:

- `HelpState::new(Focus::Explorer)` starts with an empty query and `selected == 0`.
- The Explorer catalog contains separate `Ctrl-w h`, `Ctrl-w j`, `Ctrl-w k`, and `Ctrl-w l` entries with stable, distinct IDs.
- No catalog label contains combined executable alternatives such as `h/j/k/l`, `n / e / d`, or `tc / ...`.
- Empty search preserves catalog order.
- `ctrl editor` performs case-insensitive all-token matching against key plus description.
- A query change resets selection.
- Empty matches are represented safely.

Use a shape equivalent to:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpShortcutId {
    FocusExplorer,
    FocusResults,
    FocusEditorFromK,
    FocusEditorFromL,
    // One variant per executable row.
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HelpShortcut {
    pub id: HelpShortcutId,
    pub key: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelpState {
    pub context: Focus,
    pub query: String,
    pub selected: usize,
}
```

**Step 2: Run the focused tests and verify failure**

Run: `cargo test help::tests --lib`

Expected: FAIL because `src/help.rs` and its types do not exist.

**Step 3: Implement the minimal help module**

Implement:

- `HelpState::new`, query append/backspace/clear methods, and reset behavior.
- `shortcuts(context, app capability)` or a small catalog function that accepts only the contextual facts needed by the UI.
- `filtered_shortcuts` using lowercased whitespace-separated all-token matching.
- Selection movement with wrapping and a helper to retrieve the stable selected ID.
- One catalog row for every executable shortcut currently shown by `render_help`; split every combined row.

Export the module from `src/lib.rs` and change `Overlay::Help(Focus)` to `Overlay::Help(HelpState)`.

Do not store `Action` or `KeyEvent` in the help model.

**Step 4: Run tests and formatting**

Run: `cargo fmt --check && cargo test help::tests --lib`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/help.rs src/lib.rs src/model/workspace.rs
git commit -m "feat(help): add structured shortcut catalog"
```

### Task 2: Add Help Input Actions And Reducer State Transitions

**Files:**
- Modify: `src/action.rs:25-44`
- Modify: `src/app.rs:561-580`
- Test: `src/app.rs` test module

**Step 1: Write failing reducer tests**

Add tests that assert:

- `Action::ShowHelp` creates `Overlay::Help(HelpState::new(current_focus))`.
- Character insertion, Backspace, and clear update only the help query.
- Query edits reset `selected`.
- moving selection wraps and is harmless when no results match.
- dismissing help removes the overlay.

Define narrowly scoped actions:

```rust
HelpInsert(char),
HelpPaste(String),
HelpBackspace,
HelpClear,
HelpMove(isize),
ExecuteHelpShortcut(HelpShortcutId),
```

**Step 2: Run tests and verify failure**

Run the new tests with their shared filter, for example:

`cargo test --lib help_`

Expected: FAIL because the actions and reducer branches do not exist.

**Step 3: Implement state-only reducer behavior**

Add the action variants and reducer branches. Build the capability-aware filtered list from the active tab before moving selection. Keep execution unimplemented except for safe ID validation until Task 4.

When borrowing `self.overlay`, compute contextual capability before taking a mutable overlay borrow to avoid overlapping app borrows.

**Step 4: Run focused reducer tests**

Run: `cargo fmt --check && cargo test --lib help_`

Expected: PASS for state transition tests.

**Step 5: Commit**

```bash
git add src/action.rs src/app.rs
git commit -m "feat(help): manage search and selection state"
```

### Task 3: Route Keyboard And Paste Input To Help

**Files:**
- Modify: `src/input/keymap.rs:31-135`
- Test: `tests/keymap.rs`

**Step 1: Write failing keymap tests**

Create a helper that opens help through `app.update(Action::ShowHelp)`, then assert:

- printable `q`, `j`, and `k` map to `HelpInsert` instead of dismissing or navigating.
- Up/Down map to `HelpMove(-1/1)`.
- Enter maps to `ExecuteHelpShortcut` with the currently selected stable ID.
- Backspace maps to `HelpBackspace` and `Ctrl-u` maps to `HelpClear`.
- Esc maps to `DismissOverlay`.
- release events return `None`.
- `map_paste` returns `HelpPaste` while help is open and keeps the existing behavior for other overlays.

**Step 2: Run tests and verify failure**

Run: `cargo test --test keymap help_`

Expected: FAIL because help still falls through generic overlay handling.

**Step 3: Implement help-first input mapping**

Before other overlay branches in `Keymap::map`:

- Match `Some(Overlay::Help(_))`.
- Clear `pending`.
- Ignore unsupported modifiers and release events.
- Map only the confirmed text-first controls.
- Resolve Enter from the app's current filtered selection; return `None` for no matches.

In `map_paste`, detect Help before the generic `overlay.is_some()` rejection. Normalize `\r`, `\n`, and `\t` to spaces either here or in the help state, with one canonical implementation.

**Step 4: Run keymap tests**

Run: `cargo fmt --check && cargo test --test keymap`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/input/keymap.rs tests/keymap.rs
git commit -m "feat(help): capture search and navigation input"
```

### Task 4: Execute Stable Shortcut IDs Through Existing Behavior

**Files:**
- Modify: `src/app.rs`
- Modify: `src/help.rs`
- Test: `src/app.rs` test module

**Step 1: Write failing execution tests**

Cover representative classes rather than only one row:

- `Ctrl-w h` closes help then focuses Explorer.
- `Ctrl-w k` and `Ctrl-w l` are distinct IDs but both focus Editor.
- next/previous tab IDs call existing tab actions.
- an Explorer action executes against the captured/current valid context.
- an Editor single key such as `i` closes help and enters Insert mode.
- an Editor prefix such as `d` closes help and leaves the editor waiting for the next key, matching real input.
- an action that returns runtime `Command` values propagates them from `App::update`.
- a stale or non-selected ID is ignored without closing or executing.

**Step 2: Run tests and verify failure**

Run: `cargo test --lib execute_help_`

Expected: FAIL because execution is not yet resolved.

**Step 3: Implement semantic resolution**

Add a private resolution type in `app.rs`, for example:

```rust
enum HelpExecution {
    Action(Action),
    EditorKeys(&'static [HelpEditorKey]),
}
```

Map every `HelpShortcutId` exhaustively. Use existing `Action` variants for app behavior. For editor-only rows, convert explicit help editor keys to `Action::EditorKey(KeyEvent)` and feed them through the existing reducer path in sequence.

In `ExecuteHelpShortcut`:

1. Confirm the ID equals the currently selected filtered ID.
2. Set `self.overlay = None`.
3. Apply the resolved action(s) through `self.update` and concatenate returned commands.

Do not send synthetic events back through `Keymap`; do not sleep or manipulate the pending-sequence timeout.

**Step 4: Run focused and app tests**

Run: `cargo fmt --check && cargo test --lib execute_help_ && cargo test --lib app::tests`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/help.rs src/app.rs
git commit -m "feat(help): execute selected shortcuts"
```

### Task 5: Render Search, Selection, Cursor, And Scrolling

**Files:**
- Modify: `src/ui/mod.rs:1040-1052`
- Modify: `src/ui/mod.rs:1312-1392`
- Test: `tests/ui_render.rs:732-743`

**Step 1: Write failing render tests**

Extend `help_overlay_is_contextual` and add tests that assert:

- `Search` is the first content row.
- an empty query displays a cursor at the search input and `UiState.cursor_style == Some(CursorStyle::Bar)`.
- the first shortcut has selection background cells.
- `Ctrl-w h`, `Ctrl-w j`, `Ctrl-w k`, and `Ctrl-w l` appear on separate rows.
- combined labels no longer appear.
- a query renders only matching rows.
- no matches renders `No matching shortcuts`.
- moving selection in a short popup scrolls the list while keeping the selected item visible.

Prefer assertions against backend cells/styles for selection and cursor, not only substring checks.

**Step 2: Run tests and verify failure**

Run: `cargo test --test ui_render help_`

Expected: FAIL because `render_help` is still static and receives no mutable UI state.

**Step 3: Implement the dynamic help renderer**

- Pass `&HelpState` and `&mut UiState` from `render_overlay` into `render_help`.
- Compute a popup height constrained by terminal height instead of fixed `22`.
- Split the inner area into search, spacer, list, and footer.
- Render one non-wrapping line per filtered shortcut.
- Apply `theme.selection` to the complete selected row.
- Derive the visible start from `selected` and the current list height so selection remains visible without mutating app state during render.
- Place the frame cursor after the rendered query using terminal display width and set `state.cursor_style = Some(CursorStyle::Bar)`.
- Render the fixed control footer and safe empty-result message.

Delete the old ad hoc `lines` construction and retain/rework `key_line` only if it serves the structured rows.

**Step 4: Run UI tests**

Run: `cargo fmt --check && cargo test --test ui_render help_`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(help): render searchable shortcut palette"
```

### Task 6: Align Documentation And Run Full Verification

**Files:**
- Modify: `docs/keybindings.md`
- Modify if needed: `docs/plans/2026-08-27-searchable-help-palette-design.md`

**Step 1: Update the keyboard contract**

Document:

- Search input behavior.
- Up/Down selection, Enter execution, Esc closing, and `Ctrl-u` clearing.
- Text-first handling of `q/j/k`.
- One row per executable shortcut, including four separate panel-focus rows.
- The close-before-execute behavior.

Split documentation rows that currently combine different actions, while aliases that truly perform the same atomic action may remain documented together outside the palette only if that does not imply one selectable row.

**Step 2: Run formatting and focused tests**

Run:

```bash
cargo fmt --check
cargo test --test keymap
cargo test --test ui_render help_
cargo test --lib help_
```

Expected: all commands PASS.

**Step 3: Run the complete suite and lint**

Run:

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

**Step 4: Manually smoke-test the TUI**

Run: `cargo run`

Verify:

- Open help from Explorer, Editor Normal mode, and Results.
- Type searches containing `q`, `j`, and `k`.
- Navigate past both list boundaries.
- Execute focus, tab, Explorer, Results, Editor mode, and Editor prefix shortcuts.
- Resize to a short terminal and verify search/footer remain usable and selection remains visible.

Expected: help always owns text focus; Enter closes it before the selected behavior becomes visible.

**Step 5: Commit**

```bash
git add docs/keybindings.md docs/plans/2026-08-27-searchable-help-palette-design.md
git commit -m "docs(help): document searchable shortcut palette"
```
