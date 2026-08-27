# Workspace Tab Placement Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Place workspace tabs above the main SQL/relation content, preserve one-tab-per-relation behavior, and add `Space s` to return to the first SQL console.

**Architecture:** Keep `Vec<WorkspaceTab>` and `active_tab` as the single tab model. Change only the Ratatui layout projection, route both global and editor leader input to a new reducer action, and add regression tests around the existing `RelationKey` lookup.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm 0.29, Modalkit, Cargo test, rustfmt, Clippy

---

### Task 1: Relocate the Workspace Tab Layout

**Files:**
- Modify: `src/ui/layout.rs:13-155`
- Modify: `src/ui/mod.rs:164-246`
- Test: `src/ui/layout.rs`

**Step 1: Add failing standard-layout tests**

Add a `#[cfg(test)]` module to `src/ui/layout.rs` with tests using a stable area such as `Rect::new(0, 0, 180, 50)`.

Assert the following SQL-tab geometry:

```rust
let layout = AppLayout::calculate(Rect::new(0, 0, 180, 50), Focus::Editor, false);
let explorer = layout.explorer.unwrap();
let tabs = layout.tabs.unwrap();
let editor = layout.editor.unwrap();

assert_eq!(tabs.x, explorer.right());
assert_eq!(tabs.y, explorer.y);
assert_eq!(tabs.width, editor.width);
assert_eq!(editor.y, tabs.bottom());
assert_eq!(explorer.bottom(), layout.body.bottom());
```

Change `AppLayout.tabs` from `Rect` to `Option<Rect>` so hidden focus-mode tabs are represented explicitly instead of with an empty rectangle.

**Step 2: Add failing relation and focus-layout tests**

Add tests that assert:

- A standard relation layout places `relation.y == tabs.bottom()` and keeps `relation.x == tabs.x`.
- Narrow Explorer focus has `tabs == None` and `explorer == Some(body)`.
- Narrow Editor focus has `tabs == Some(...)`, with Editor immediately below it.
- Narrow Results focus has `tabs == Some(...)`, with result tabs and Results below it.
- The too-small layout has `tabs == None`.

Use an area width below 100 but above the minimum, for example `Rect::new(0, 0, 90, 40)`, for focus-mode tests.

**Step 3: Run tests and verify the old layout fails**

Run:

```bash
cargo test ui::layout --all-features
```

Expected: compilation or assertions fail because `tabs` is currently a full-width `Rect` allocated above `body`.

**Step 4: Implement the body-first split**

In `AppLayout`:

```rust
pub tabs: Option<Rect>,
```

Change the outer vertical split to allocate only header, body, and footer:

```rust
let vertical = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Length(2),
        Constraint::Min(8),
        Constraint::Length(2),
    ])
    .split(area);
```

For standard/wide mode:

1. Split `body` into Explorer and main columns.
2. Split the main column into `Constraint::Length(2)` for tabs and `Constraint::Min(...)` for active content.
3. If `is_relation`, assign all active content below tabs to `relation`.
4. Otherwise split active content into Editor, result tabs, and Results using the existing 46% / 2-row / minimum proportions.

For narrow focus mode:

- Explorer: `tabs: None`, `explorer: Some(body)`.
- Editor: split `body` into tabs and SQL content; expose Editor below tabs.
- Results for SQL: split `body` into tabs, result-view tabs, and Results.
- Results for relation: split `body` into tabs and relation content.

Do not allocate hidden SQL panes merely to calculate their positions.

**Step 5: Render optional tabs**

In `render_with_state`, replace the unconditional call:

```rust
render_tabs(frame, layout.tabs, app, theme, state);
```

with:

```rust
if let Some(area) = layout.tabs {
    render_tabs(frame, area, app, theme, state);
}
```

Keep the existing active page branching and mouse hit-region generation.

**Step 6: Run focused tests**

Run:

```bash
cargo test ui::layout --all-features
```

Expected: all new layout tests pass.

### Task 2: Align Tab Rendering and Mouse Hit Regions

**Files:**
- Modify: `src/ui/mod.rs:427-462`
- Test: `src/ui/mod.rs`
- Test: `src/input/mouse.rs`

**Step 1: Add a failing render/hit-region test**

Create a Ratatui `TestBackend`, render an `App` with at least two SQL consoles through `render_with_state`, and inspect `UiState.hit_regions`.

Assert every `HitTarget::Tab(_)` region starts at or to the right of the Explorer's right boundary. Also assert the rendered main tab row contains `01 console` and does not contain `WORKSPACE`.

Use the same terminal dimensions as the standard layout test so expected coordinates are deterministic.

**Step 2: Run the test and verify it fails**

Run the exact test by name:

```bash
cargo test workspace_tabs_render_inside_main_column --all-features
```

Expected: failure because the current row starts at terminal `x` and includes `WORKSPACE`.

**Step 3: Remove the workspace prefix and recalculate the first hit position**

Change `render_tabs` to begin with an empty span list and initialize `x` at `area.x`:

```rust
let mut spans = Vec::new();
let mut x = area.x;
```

Keep title sanitization, 48-character bounding, sequence numbers, active style, and `HitTarget::Tab(index)` unchanged.

When a tab would begin beyond `area.right()`, stop adding hit regions. Clamp visible hit widths to the remaining row width so hidden or clipped labels are not clickable outside the main column.

**Step 4: Verify mouse activation still uses semantic tab indices**

Add or extend a mouse unit test that clicks the first relocated tab hit region and asserts:

```rust
assert_eq!(action, Some(Action::ActivateTab(0)));
```

No change should be required in `src/input/mouse.rs`; this test protects the render-to-input contract.

**Step 5: Run UI and mouse tests**

Run:

```bash
cargo test ui:: --all-features
cargo test input::mouse --all-features
```

Expected: all tests pass and tab hit regions remain inside the main column.

### Task 3: Lock In Relation Tab Reuse

**Files:**
- Test: `src/app.rs:4602-4887`
- Reference: `src/app.rs:4061-4122`
- Reference: `src/model/relation.rs:13-18`

**Step 1: Add catalog test setup**

Inside the existing `app.rs` test module, add a small helper that creates a profile and inserts a relation plus a supported child into that profile's normalized catalog. Return the profile UUID, relation `CatalogId`, and child `CatalogId` so each test can select either node.

Use existing `CatalogEntry`/catalog insertion helpers rather than constructing application state that bypasses catalog validation.

**Step 2: Add a failing explicit reuse test**

Write a reducer test that:

1. Selects a table relation.
2. Calls `Action::OpenSelectedRelation { view: RelationView::Data }`.
3. Records the created relation tab UUID.
4. Switches to another tab.
5. Selects the same relation and opens it again.
6. Asserts there is exactly one matching relation tab, it is active, and its UUID is unchanged.

This may already pass; it is a characterization test for the required behavior.

**Step 3: Add owning-relation and view-switch tests**

Open the relation through a child node after its relation tab already exists. Assert the same tab is activated. Then request `RelationView::Structure` and assert the reused tab switches its `view` without adding another tab.

**Step 4: Add cross-profile identity test**

Create two profiles with equally named relations but different profile UUIDs and catalog IDs. Open both and assert two relation tabs exist. This proves `RelationKey` includes profile identity and avoids name-based collisions.

**Step 5: Run focused reducer tests**

Run each new test by name, then:

```bash
cargo test app::tests --all-features
```

Expected: all relation reuse tests pass without changing production lookup logic. If a test exposes a real gap, make the smallest change inside `open_selected_relation`; do not introduce a second registry.

### Task 4: Add the Goto SQL Console Reducer Action

**Files:**
- Modify: `src/action.rs:24-35`
- Modify: `src/app.rs:346-470`
- Test: `src/app.rs:4602-4887`

**Step 1: Write failing reducer tests**

Add tests for these cases:

- With SQL, SQL, Relation tabs and Relation active, `GotoSqlConsole` activates the first SQL tab and sets `focus` to `Focus::Editor`.
- With a later SQL tab active and `focus == Focus::Results`, it activates the first SQL tab and focuses Editor.
- After closing the original startup console, it activates the first remaining SQL console.
- In a defensively constructed tab vector containing no SQL tab, it performs no state mutation and does not panic.

**Step 2: Run tests and verify the action is missing**

Run:

```bash
cargo test goto_sql_console --all-features
```

Expected: compilation fails because `Action::GotoSqlConsole` does not exist.

**Step 3: Add and handle the semantic action**

Add this variant next to tab navigation actions:

```rust
GotoSqlConsole,
```

Handle it in `App::update` without emitting commands:

```rust
Action::GotoSqlConsole => {
    if let Some(index) = self.tabs.iter().position(|tab| tab.as_console().is_some()) {
        self.active_tab = index;
        self.focus = Focus::Editor;
    }
    Vec::new()
}
```

Do not key this behavior by title, display sequence number, or persisted UUID. “01” means the first available SQL console in current tab order.

**Step 4: Run reducer tests**

Run:

```bash
cargo test goto_sql_console --all-features
cargo test app::tests --all-features
```

Expected: all new and existing reducer tests pass.

### Task 5: Map `Space s` Through Both Input Paths

**Files:**
- Modify: `src/input/keymap.rs:283-304`
- Modify: `src/editor/mod.rs:39-60`
- Modify: `src/editor/mod.rs:1063-1091`
- Modify: `src/app.rs:2925-2947`
- Test: `src/editor/tests.rs`
- Test: `src/input/keymap.rs`

**Step 1: Write failing global-keymap tests**

Add keymap tests that send `Space`, then `s`, and assert `Action::GotoSqlConsole` from:

- Explorer focus.
- SQL Results focus.
- Relation Results focus with no relation query input selected.

Add negative tests asserting it is not emitted while an overlay is active or while a relation `WHERE`/`ORDER BY` input owns keyboard input.

**Step 2: Write failing Editor Normal/Insert tests**

In `src/editor/tests.rs`, add a test that sends `Space`, then `s`, to an Editor Normal session and asserts the emitted effect is converted to `Action::GotoSqlConsole` by App processing.

Add an Insert-mode test that enters `" s"` as editor text and emits no goto effect. This ensures the shortcut does not steal normal SQL input.

**Step 3: Run tests and verify failures**

Run:

```bash
cargo test goto_sql_console --all-features
cargo test editor::tests --all-features
```

Expected: failures because neither leader state machine recognizes `s`.

**Step 4: Add the global leader mapping**

In `map_pending` add:

```rust
(Pending::Leader, KeyCode::Char('s')) => Some(Action::GotoSqlConsole),
```

Retain the current ordering where overlays and relation query input are handled before global pending sequences. This gives text input and modal overlays priority.

**Step 5: Add the editor leader effect**

Add:

```rust
GotoSqlConsole,
```

to `EditorEffect`, then map Editor Normal leader `s`:

```rust
(PendingBinding::Leader, 's') => self.effects.push(EditorEffect::GotoSqlConsole),
```

Convert the effect in App's editor-effect dispatch:

```rust
EditorEffect::GotoSqlConsole => Action::GotoSqlConsole,
```

Do not add special handling to Insert mode; its existing editor path must continue inserting characters.

**Step 6: Run input and editor tests**

Run:

```bash
cargo test input::keymap --all-features
cargo test editor::tests --all-features
cargo test goto_sql_console --all-features
```

Expected: shortcut tests pass in all supported contexts and negative precedence tests remain green.

### Task 6: Update Footer, Help, and Documentation

**Files:**
- Modify: `src/ui/mod.rs:1032-1084`
- Modify: `src/ui/mod.rs:1355-1418`
- Modify: `docs/keybindings.md:7-22`
- Modify: `docs/architecture.md:112-128`

**Step 1: Add the contextual help entry**

Add a global help line near new-console and tab navigation entries:

```rust
key_line("Space s", "go to first SQL console", theme),
```

Ensure the popup height still accommodates the line. If necessary, increase only the help popup's bounded height by one row.

**Step 2: Add relation footer guidance**

When the active tab is a relation tab, include `Space s SQL console` in the Results footer hints. Keep the ordinary SQL Results footer unchanged to avoid persistent hint overload.

Use active tab kind rather than focus alone when selecting the hint string.

**Step 3: Update keybinding documentation**

Add to the Global table in `docs/keybindings.md`:

```markdown
| `Space s` | Go to the first available SQL console and focus its editor |
```

Clarify that if the startup console has been closed, “first” means the earliest remaining SQL console in tab order.

**Step 4: Update architecture wording**

In the Rendering Boundary section, state that heterogeneous workspace tabs live above and select only the main content column; Explorer is outside their visual and state-selection scope.

Retain the existing description of relation focus behavior and SQL-only actions.

**Step 5: Run UI tests**

Run:

```bash
cargo test ui:: --all-features
```

Expected: contextual help and render tests pass.

### Task 7: Complete Verification

**Files:**
- Verify: `src/action.rs`
- Verify: `src/app.rs`
- Verify: `src/editor/mod.rs`
- Verify: `src/input/keymap.rs`
- Verify: `src/ui/layout.rs`
- Verify: `src/ui/mod.rs`
- Verify: `docs/keybindings.md`
- Verify: `docs/architecture.md`

**Step 1: Format and inspect formatting changes**

Run:

```bash
cargo fmt --all -- --check
```

If it fails, run `cargo fmt --all`, then rerun the check.

Expected: formatting check succeeds.

**Step 2: Run the complete test suite**

Run:

```bash
cargo test --all-features --all-targets
```

Expected: all tests pass.

**Step 3: Run strict Clippy**

Run:

```bash
cargo clippy --all-features --all-targets -- -D warnings
```

Expected: no warnings or errors.

**Step 4: Inspect the final diff**

Run:

```bash
git status --short
```

Expected: only intended files are changed, `git diff --check` reports no whitespace errors, and no runtime/database/persistence behavior was modified.

**Step 5: Manual acceptance check**

Launch LazyDB in a terminal wide enough for standard layout and verify:

1. Explorer begins under the full-width header and extends through the old tab-row height.
2. Tabs begin at Explorer's right edge and sit above SQL Editor or relation preview.
3. Opening the same table twice reuses its tab.
4. Opening a second table adds a separate tab.
5. `Space s` from a relation preview returns to the first SQL console and places focus in Editor.
6. `Space s` typed in Editor Insert mode remains SQL text.

Do not create a commit unless the user explicitly requests one.
