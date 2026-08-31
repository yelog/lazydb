# Read-Only Vim Copy Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add semantic Explorer copy and full read-only Vim navigation, Visual selection, and clipboard yank support to SQL Output Log and relation DDL.

**Architecture:** Extend `EditorWorkspace` with explicit editable and read-only session capabilities. Keep Output Log entries and adapter-owned DDL as authoritative domain data, project them into per-view read-only sessions, and reuse the existing editor render snapshot and clipboard effect path. Route existing view-switching and global shortcuts before read-only Vim input.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm 0.29, Modalkit 0.0.25, existing reducer/command runtime, Cargo test/clippy/fmt.

**Design reference:** `docs/plans/2026-08-31-read-only-vim-copy-design.md`

**Commit policy:** The steps below identify logical commit checkpoints, but do not commit unless the user explicitly authorizes commits.

---

### Task 1: Add Strongly Read-Only Editor Sessions

**Files:**
- Modify: `src/editor/mod.rs:158-286, 809-873, 1105-1236`
- Modify: `src/editor/tests.rs:1-end`
- Modify: `src/model/editor.rs:1-99` only if a public session-capability or interaction snapshot type is needed outside `editor`

**Step 1: Write failing read-only session contract tests**

Add a `read_only_fixture` next to the existing editor test fixtures. Open the session directly in Normal mode and cover these contracts:

```rust
#[test]
fn read_only_session_supports_navigation_visual_modes_and_yank() {
    let (mut workspace, id) = read_only_fixture("alpha beta\ngamma delta");

    press_keys(&mut workspace, id, "wve");
    workspace.press(id, EditorKey::Character('y')).unwrap();

    assert_eq!(workspace.text(id).unwrap(), "alpha beta\ngamma delta");
    assert_eq!(workspace.register('"'), Some("beta"));
    assert!(matches!(
        workspace.drain_effects().as_slice(),
        [EditorEffect::Yanked(text)] if text == "beta"
    ));
}
```

Add table-driven cases for `v`, `V`, `Ctrl-v`, `yy`, `y$`, `gg/G`, `H/M/L`, and `Ctrl-u/d/f/b`. Assert cursor/mode/selection and copied register values, not only lack of panic.

**Step 2: Write failing immutability tests**

Run representative mutation sequences against one read-only fixture:

```rust
for sequence in ["iX<Esc>", "aX<Esc>", "oX<Esc>", "x", "dw", "cwX<Esc>", "p", "u", ".", "rX"] {
    // Create a fresh fixture, press parsed keys, and assert text and revision are unchanged.
}
```

Also cover `:s/a/b/`, paste APIs intended only for editable sessions, and Insert/Replace entry. Assert the mode remains Normal or returns to Normal, no `EditorEffect::Changed` is emitted, and revision remains zero.

**Step 3: Run the focused tests and verify they fail**

Run:

```text
cargo test editor::tests::read_only --lib
```

Expected: FAIL because no read-only session API or capability exists.

**Step 4: Add the session capability and constructor**

In `src/editor/mod.rs`, add a private or crate-visible capability owned by every `EditorSession`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditorSessionCapability {
    Editable,
    ReadOnly,
}
```

Refactor construction into one internal `open_session(id, text, capability, initial_mode)` path. Keep `open_console` behavior unchanged and add:

```rust
pub(crate) fn open_read_only(&mut self, id: Uuid, text: &str);
```

Read-only sessions must start in Normal mode. Do not simulate Insert and Escape when constructing them.

**Step 5: Enforce read-only before editor actions execute**

Add an exhaustive action classifier at the `apply_action` boundary. For `Action::Editor(editor_action)`, inspect the Modalkit editor action and skip every action that can mutate buffer text, undo history, or editable document state when the session capability is `ReadOnly`. Continue to execute cursor, viewport, selection, and yank/register actions.

Do not implement this as “execute then restore text.” Tests must prove revision and undo state remain untouched.

At `press`, disable SQL-only special handling and effects for read-only sessions:

- Do not enter Insert or Replace.
- Do not run undo/redo or dot-repeat edits.
- Do not open command prompts that can substitute or mutate.
- Preserve `/`, `?`, `n`, `N`, Visual mode, motions, and yank.
- Preserve pane/tab effects only where the top-level keymap does not already own them.
- Suppress Run, RunAll, Format, transaction, target, and console-lifecycle effects.

Keep Visual yank detection and `EditorEffect::Yanked` shared with editable sessions. Extend detection beyond only `was_visual && key == 'y'` so `yy` and yank-plus-motion sequences also emit the clipboard effect when the unnamed register changes due to a yank.

**Step 6: Add derived-text replacement and interaction state APIs**

Add APIs that update read-only content without treating the update as a user edit:

```rust
pub(crate) enum ReadOnlyUpdate {
    Replace,
    AppendFollowTail,
}

pub(crate) fn set_read_only_text(
    &mut self,
    id: Uuid,
    text: &str,
    update: ReadOnlyUpdate,
) -> Result<(), EditorError>;

pub(crate) fn read_only_interacted(&self, id: Uuid) -> Result<bool, EditorError>;
```

Track whether a read-only session has received navigation, selection, search, or explicit scroll input. For `AppendFollowTail`, move to the new end only while untouched; otherwise preserve cursor, selection, and viewport. For replacement, retain valid coordinates and clamp cursor and viewport to the new line/column bounds. Derived updates must not emit `Changed` or increment the editable document revision.

**Step 7: Run focused editor tests**

Run:

```text
cargo test editor::tests --lib
```

Expected: PASS, including existing editable SQL Editor behavior.

**Step 8: Logical commit checkpoint**

If explicitly authorized:

```text
git add src/editor/mod.rs src/editor/tests.rs src/model/editor.rs
git commit -m "feat(editor): add read-only vim sessions"
```

---

### Task 2: Add Semantic Explorer Node Copy

**Files:**
- Modify: `src/action.rs` near Explorer actions and clipboard actions
- Modify: `src/model/explorer.rs:1211-1217` and nearby node lookup helpers
- Modify: `src/model/workspace.rs:1104-1181` or move/reuse its label helpers
- Modify: `src/input/keymap.rs:213-285` and the normal Explorer branch
- Modify: `src/app.rs:985-1121, 1228-2300`
- Modify: `tests/keymap.rs`
- Modify: `tests/explorer_state.rs`
- Modify: `tests/app_flow.rs` or add reducer coverage in the existing `src/app.rs` test module

**Step 1: Write failing label-resolution tests**

Add tests for a pure Explorer API such as:

```rust
pub fn selected_copy_name(&self) -> Option<String>;
```

Cover profile, group, database/schema, table/view, and column nodes. Cover status, empty, and load-more nodes returning `None`. Reuse `CatalogEntry.qualified_name.object` and the existing group-label mapping rather than parsing rendered lines.

**Step 2: Write failing keymap tests**

Assert that plain `y` in normal Explorer focus maps to `Action::CopyExplorerSelection`. Assert that `y` in active Explorer find/search input remains query input. Assert Control/Alt modified `y` does not trigger semantic copy.

**Step 3: Write failing reducer tests**

For a named selected node, expect one command:

```rust
Command::WriteClipboard(ClipboardPayload {
    text: "users".into(),
    description: "Explorer name: users".into(),
    sensitive: false,
})
```

For an operational row, expect no command and no clipboard replacement.

**Step 4: Run focused tests and verify failure**

Run:

```text
cargo test --test explorer_state
cargo test --test keymap explorer
cargo test --test app_flow clipboard
```

Expected: FAIL because the action and semantic-name API do not exist.

**Step 5: Implement semantic lookup and action routing**

Add `Action::CopyExplorerSelection`. Resolve the selected node through normalized Explorer state and return only its semantic primary name. Consolidate `group_label`, `entry_label`, and profile-name lookup so UI projection and clipboard semantics cannot drift.

Map `y` only after active Explorer search input handling and before unrelated normal Explorer commands. In `App::update`, create `Command::WriteClipboard` only when a name exists.

**Step 6: Run focused tests**

Run:

```text
cargo test --test explorer_state
cargo test --test keymap
cargo test --test app_flow
```

Expected: PASS.

**Step 7: Logical commit checkpoint**

If explicitly authorized:

```text
git add src/action.rs src/model/explorer.rs src/model/workspace.rs src/input/keymap.rs src/app.rs tests/explorer_state.rs tests/keymap.rs tests/app_flow.rs
git commit -m "feat(explorer): copy selected node names"
```

---

### Task 3: Project SQL Output Log Into a Read-Only Vim Session

**Files:**
- Modify: `src/model/tab.rs:124-210`
- Modify: `src/action.rs` near editor viewport/key actions
- Modify: `src/app.rs` at all `tab.output.push(...)` sites and editor effect handling
- Modify: `src/input/keymap.rs:324-end`
- Modify: `src/input/mouse.rs:130-175`
- Modify: `src/ui/mod.rs:1279-1663`
- Modify: `tests/keymap.rs`
- Modify: `tests/mouse.rs`
- Modify: `tests/sql_execution.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Add failing output projection and lifecycle tests**

Add a stable `output_editor_id: Uuid` to `ConsoleTab` and tests proving every console's editable SQL session and Output session are distinct. Add a pure projection helper that joins `OutputEntry.message` in append order while preserving embedded newlines and excluding status markers.

Test opening a console creates both sessions, hiding/reopening a console recreates the transient Output session from current tab output, and permanent deletion closes both sessions.

**Step 2: Add failing append/follow-tail reducer tests**

Cover:

- Initial output append updates the read-only buffer and leaves the cursor at the newest content.
- Multiple entries remain in append order.
- After an Output navigation action, appending an entry preserves cursor, selection, and viewport.
- Output text contains message text only, not `·/✓/!/×`.

Create one App helper and route every current `tab.output.push(...)` site through it:

```rust
fn push_console_output(&mut self, console_id: Uuid, entry: OutputEntry);
```

This avoids missing synchronization in transaction, connection, cancellation, and execution paths.

**Step 3: Add failing keymap tests**

When `Focus::Results` and `ResultView::Output` are active:

- Preserve `o`, `1`, `2`, and `3` result-page commands.
- Route remaining supported key events to a new `Action::ReadOnlyEditorKey { session_id, event }` or a domain-specific equivalent.
- Ensure Data-grid copy and navigation actions do not fire on Output.
- Route `Ctrl-u/d/f/b`, arrows, and Visual/yank keys.

**Step 4: Add failing output render tests**

In `tests/ui_render.rs`, render an Output session with multiline messages and assert:

- The cursor appears only when Results/Output has focus.
- Visual selection cells use the SQL Editor selection style.
- Status markers retain per-entry kind styling but are not in the selected buffer text.
- Horizontal and vertical viewport offsets affect visible text.
- Empty output retains “No execution output” and does not create selectable placeholder text.

**Step 5: Run focused tests and verify failure**

Run:

```text
cargo test --test sql_execution output
cargo test --test keymap output
cargo test --test mouse output
cargo test --test ui_render output
```

Expected: FAIL because Output has no session, key routing, or editor renderer.

**Step 6: Implement Output session ownership and synchronization**

Add `output_editor_id` to `ConsoleTab`. Open the corresponding read-only session wherever a console tab is created or restored. Close it when the transient tab is closed and recreate it if reopened; close it permanently when the console is deleted.

Replace direct output pushes with `push_console_output`, then synchronize the complete projection through `ReadOnlyUpdate::AppendFollowTail`. Favor correctness over incremental string bookkeeping; output is already bounded by in-memory tab lifetime and can be optimized later only if profiling shows a need.

Drain `EditorEffect::Yanked(text)` through the existing `Action::CopyEditorYank(text)` and `Command::WriteClipboard` path.

**Step 7: Extract and reuse editor snapshot rendering**

Refactor the body of `render_editor` into a helper that draws an `EditorRenderSnapshot` with configurable:

- Border/title.
- Base text styles or per-line style callback.
- Whether SQL highlighting, completion, prompt, and editor status are enabled.
- Whether marker/gutter decoration is drawn outside the selectable text area.

Use the helper from both SQL Editor and Output. For Output, map projected buffer lines back to `OutputKind` for marker/color decoration. Embedded message lines inherit their owning entry kind. Pass the inner text viewport dimensions back through `UiState` so `App` can call `set_viewport` for the Output session.

**Step 8: Implement mouse scroll routing**

Replace Output's absent/static scrolling behavior with the read-only session scroll action. Keep Shift-drag terminal-native selection behavior unchanged.

**Step 9: Run focused tests**

Run:

```text
cargo test editor::tests --lib
cargo test --test sql_execution
cargo test --test keymap
cargo test --test mouse
cargo test --test ui_render
```

Expected: PASS.

**Step 10: Logical commit checkpoint**

If explicitly authorized:

```text
git add src/model/tab.rs src/action.rs src/app.rs src/input/keymap.rs src/input/mouse.rs src/ui/mod.rs tests/keymap.rs tests/mouse.rs tests/sql_execution.rs tests/ui_render.rs
git commit -m "feat(output): add read-only vim navigation and copy"
```

---

### Task 4: Replace Relation DDL Scroll State With a Read-Only Vim Session

**Files:**
- Modify: `src/model/relation.rs:122-290`
- Modify: `src/action.rs` DDL scroll and editor actions
- Modify: `src/app.rs:1450-1465, 7650-7840` and DDL action handling
- Modify: `src/input/keymap.rs:391-401, 792-826`
- Modify: `src/input/mouse.rs:130-225`
- Modify: `src/ui/relation.rs:283-end`
- Modify: `tests/relation_tabs.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/mouse.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Write failing DDL session lifecycle tests**

Replace `DdlViewportState` expectations with a stable `ddl_editor_id: Uuid` on `RelationTab`. Test that new/restored relation tabs create distinct read-only session identifiers and that closing a relation tab closes its DDL session.

**Step 2: Write failing DDL load/refresh tests**

Cover successful DDL loading projecting the exact adapter-owned `RelationDdl.sql`. Then position the cursor/viewport near the old end, load shorter DDL, and assert cursor and viewport clamp to valid positions. Cover loading, failed, and cancelled states retaining a previous snapshot without replacing its selectable text with status text.

**Step 3: Write failing keymap and mouse tests**

When relation DDL is active and Results has focus:

- Keep `o`, `1`, `2`, `r`, and cancellation behavior ahead of Vim input.
- Route `h/j/k/l`, arrows, `gg/G`, `H/M/L`, `Ctrl-u/d/f/b`, search, Visual modes, and yank to the DDL session.
- Remove expectations for `Action::DdlScroll`, `DdlScrollToStart`, and `DdlScrollToEnd` where the read-only editor now owns navigation.
- Route mouse wheel to the DDL read-only editor viewport by three rows.

**Step 4: Write failing DDL UI tests**

Render representative PostgreSQL/MySQL/SQLite DDL and assert:

- Existing SQL dialect highlighting remains applied to visible text.
- Cursor and Visual selection are visible only while DDL/Results has focus.
- Status and provenance remain outside selectable text.
- Long DDL lines use the editor horizontal offset.
- Wide Unicode comments align cursor and selection correctly.

**Step 5: Run focused tests and verify failure**

Run:

```text
cargo test --test relation_tabs ddl
cargo test --test keymap ddl
cargo test --test mouse ddl
cargo test --test ui_render ddl
```

Expected: FAIL because DDL still uses `DdlViewportState` and bespoke scroll actions.

**Step 6: Implement DDL session ownership and projection**

Add `ddl_editor_id` to `RelationTab`, create its read-only session whenever a relation tab is opened/restored, and close it when the tab is removed. On successful `RelationRequestKind::Ddl`, call `set_read_only_text(..., ReadOnlyUpdate::Replace)` with the exact `snapshot.sql`. For loading/failure/cancellation with a previous snapshot, keep the existing projected text unchanged.

Remove `DdlViewportState` once no call site depends on it. Remove obsolete DDL scroll actions and reducers instead of retaining backward-compatibility aliases; these are internal actions with no persisted or external consumer.

**Step 7: Reuse the editor snapshot renderer for DDL**

Replace `render_ddl_body` viewport slicing with the shared editor snapshot renderer introduced in Task 3. Keep dialect highlighting by requesting the DDL session snapshot with `app.sql_dialect()`. Draw loading/error/cancel status in its existing separate area and report the actual text viewport through `UiState`.

**Step 8: Run focused tests**

Run:

```text
cargo test --test relation_tabs
cargo test --test keymap
cargo test --test mouse
cargo test --test ui_render
```

Expected: PASS.

**Step 9: Logical commit checkpoint**

If explicitly authorized:

```text
git add src/model/relation.rs src/action.rs src/app.rs src/input/keymap.rs src/input/mouse.rs src/ui/relation.rs tests/relation_tabs.rs tests/keymap.rs tests/mouse.rs tests/ui_render.rs
git commit -m "feat(ddl): add read-only vim navigation and copy"
```

---

### Task 5: Complete Clipboard Feedback, Help, and Regression Coverage

**Files:**
- Modify: `src/help.rs`
- Modify: `src/ui/mod.rs` footer/help labels as needed
- Modify: `docs/keybindings.md:66-115, 164-224`
- Modify: `README.md` only if its feature summary enumerates copy behavior
- Modify: `tests/keymap.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Add failing help contract tests**

Extend existing help/keymap tests to require discoverable entries for:

- Explorer `y`: copy selected node name.
- Output/DDL Normal navigation (`hjkl`, `HML`, page motions).
- Output/DDL Visual and Visual Line selection.
- `y`/`yy`: copy selection/current line.

Do not list unsupported mutation commands or imply that Output/DDL is editable.

**Step 2: Normalize clipboard descriptions and notices**

Ensure Explorer, Output, and DDL copies all produce non-sensitive `ClipboardPayload` values and use the existing success/failure notices. Keep the selection intact after both successful and failed writes. Avoid introducing direct `arboard` calls in App, editor, or UI code.

**Step 3: Update documentation**

In `docs/keybindings.md`:

- Add Explorer `y` and define “primary name.”
- Split SQL Results Output behavior from Data-grid behavior where necessary.
- Replace relation DDL's old bespoke scrolling description with read-only Vim cursor, page motion, Visual, Visual Line, and yank behavior.
- State that status markers, DDL provenance, and other decorations are not copied.
- State that all mutation commands are disabled in read-only views.

**Step 4: Run help and UI tests**

Run:

```text
cargo test --test keymap
cargo test --test ui_render
```

Expected: PASS.

**Step 5: Run formatting**

Run:

```text
cargo fmt --all
cargo fmt --check
```

Expected: both commands exit successfully and `cargo fmt --check` has no diff.

**Step 6: Run full static analysis**

Run:

```text
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

**Step 7: Run the full test suite**

Run:

```text
cargo test --all-targets
```

Expected: PASS. Environment-gated PostgreSQL/MySQL tests may report documented skips when their URLs are absent; do not claim they ran if skipped.

**Step 8: Inspect the final diff**

Run:

```text
git status --short
git diff --check
git diff --stat
```

Expected: only intended source, test, and documentation files are changed; `git diff --check` exits successfully.

**Step 9: Logical final commit checkpoint**

If explicitly authorized:

```text
git add src/help.rs src/ui/mod.rs docs/keybindings.md README.md tests/keymap.rs tests/ui_render.rs
git commit -m "docs: document read-only vim copy controls"
```

Do not stage `README.md` if no README change was necessary.
