# Contextual Key Hints Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Show context-aware common shortcuts in the footer, replace them with valid continuations while a key prefix is pending, and move all concrete keyboard documentation out of README into the dedicated reference.

**Architecture:** Keep `Keymap` as the executable input authority and expand `src/help.rs` into a shared display catalog consumed by contextual help and the footer. Expose a validated, read-only pending-sequence snapshot from `Keymap`, pass it through the runtime render boundary, and trigger redraws when that snapshot starts, changes, clears, or expires.

**Tech Stack:** Rust 2024, Crossterm key events, Ratatui rendering and `TestBackend`, Tokio event loop, existing LazyDB unit and integration tests.

---

### Task 1: Introduce Explicit Shortcut Display Contexts

**Files:**
- Modify: `src/help.rs:1-162`
- Modify: `src/app.rs` near existing active tab, editor mode, relation mode, and overlay query methods
- Test: `src/help.rs` test module

**Step 1: Write failing context-resolution tests**

Add table-driven tests that construct or mutate `App` fixtures and verify a new
`shortcut_context(&App)` function distinguishes at least:

```rust
#[test]
fn shortcut_context_distinguishes_results_views() {
    let mut app = sql_app_fixture();
    app.focus = Focus::Results;

    app.active_console_mut().result_view = ResultView::Data;
    assert_eq!(shortcut_context(&app), ShortcutContext::SqlResultsData);

    app.active_console_mut().result_view = ResultView::Output;
    assert_eq!(shortcut_context(&app), ShortcutContext::SqlOutput);
}
```

Also cover Explorer, Editor Normal, Editor Insert, Editor Visual, Relation Data,
Relation DDL, Record View, Data Query input, Profile Manager pages, SQL Editor
List, Help, and representative confirmation overlays.

Prefer existing test fixture helpers. If the helpers are private to another
module, construct only the minimum `App` state needed for each assertion.

**Step 2: Run the tests to verify they fail**

Run: `cargo test --lib help::tests::shortcut_context -- --nocapture`

Expected: FAIL because `ShortcutContext` and `shortcut_context` do not exist.

**Step 3: Add the context model and resolver**

Add a focused enum in `src/help.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutContext {
    Explorer,
    EditorNormal,
    EditorInsert,
    EditorVisual,
    SqlResultsData,
    SqlOutput,
    RelationDataBrowse,
    RelationDataEdit,
    RelationDataVisual,
    RelationDdl,
    RecordView,
    DataQueryInput,
    ProfileManagerForm,
    ProfileManagerScope,
    ProfileManagerDelete,
    SqlEditorList,
    Help,
    Confirmation,
}
```

Implement `shortcut_context(&App)` with the precedence approved in the design:
overlay, focused input/edit state, tab/view, then focus/editor mode. Add minimal
read-only `App` query methods only where private state prevents this resolver
from making the distinction. Do not expose mutable internals.

Treat all Visual editor variants as `EditorVisual`, and Insert plus Replace as
`EditorInsert` unless a real display difference is needed.

**Step 4: Run the focused tests**

Run: `cargo test --lib help::tests::shortcut_context -- --nocapture`

Expected: PASS.

**Step 5: Run existing help and app tests**

Run: `cargo test --lib help::tests app::tests -- --nocapture`

Expected: PASS with no existing help behavior regression.

**Step 6: Commit**

```bash
git add src/help.rs src/app.rs
git commit -m "refactor(input): model shortcut display contexts"
```

### Task 2: Convert Help Rows into a Shared Shortcut Catalog

**Files:**
- Modify: `src/help.rs:3-617`
- Modify: `src/app.rs` around `help_selected_id` and `execute_help_shortcut`
- Test: `src/help.rs` test module
- Test: `tests/ui_render.rs` help rendering tests

**Step 1: Write failing catalog integrity tests**

Add tests for unique IDs, complete sequences, descriptions, and context
selection. The catalog should retain executable IDs for help rows while adding
display metadata:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Shortcut {
    pub id: HelpShortcutId,
    pub contexts: &'static [ShortcutContext],
    pub sequence: &'static str,
    pub description: &'static str,
    pub footer_priority: Option<u8>,
    pub prefix: Option<ShortcutPrefix>,
    pub suffix: Option<&'static str>,
}
```

The exact representation may use a context bitset or helper predicate if that
keeps static declarations concise. Avoid closures and a new binding DSL.

Test that:

- Every `HelpShortcutId` appears at most once per context.
- Every prefixed row has both `prefix` and `suffix`.
- Its `sequence` ends with the declared suffix.
- Empty sequences and descriptions are rejected.
- Existing Explorer, Editor, and Results help rows still appear.

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib help::tests -- --nocapture`

Expected: FAIL because the catalog metadata does not exist.

**Step 3: Implement the shared catalog**

Replace the large imperative `shortcuts(Focus, relation_data)` construction with
static or small composable catalog slices. Preserve current help ordering where
it remains useful and add missing context-specific rows incrementally.

Keep compatibility functions only where they have current callers. Prefer a
new context API:

```rust
pub fn shortcuts_for(app: &App) -> Vec<Shortcut>;
pub fn filtered_shortcuts_for(app: &App, query: &str) -> Vec<Shortcut>;
```

Update `HelpState` to retain the context needed to keep the help overlay stable
after it opens. If an `App` reference is inappropriate inside `HelpState`, store
`ShortcutContext` plus small capability flags captured at open time rather than
the old `Focus` and `relation_data` pair.

Do not attempt to execute catalog rows directly. Continue mapping stable IDs to
existing `Action`s in `App`.

**Step 4: Update help rendering and selection callers**

Update `src/app.rs` and `src/ui/mod.rs` callers to consume the shared catalog
field names (`sequence` rather than `key`) and new context snapshot.

**Step 5: Run focused tests**

Run: `cargo test --lib help::tests -- --nocapture`

Run: `cargo test --test ui_render editor_help_documents_target_context_controls -- --nocapture`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/help.rs src/app.rs src/ui/mod.rs tests/ui_render.rs
git commit -m "refactor(help): share contextual shortcut catalog"
```

### Task 3: Add Context-Specific Default Footer Entries

**Files:**
- Modify: `src/help.rs`
- Test: `src/help.rs` test module

**Step 1: Write failing tests for footer priorities**

Add tests that select catalog rows with `footer_priority.is_some()` and assert
the important entries and order for:

- Explorer.
- Editor Normal.
- Editor Insert/Replace.
- Editor Visual.
- SQL Results Data.
- SQL Output.
- Relation Data browse/edit/Visual Line.
- Relation DDL.
- Record View.
- Data Query input.
- Profile Manager and confirmation overlays.

Example:

```rust
#[test]
fn sql_output_footer_describes_text_navigation_not_cells() {
    let rows = footer_shortcuts(ShortcutContext::SqlOutput, ShortcutCapabilities::default());
    let sequences = rows.iter().map(|row| row.sequence).collect::<Vec<_>>();

    assert!(sequences.contains(&"gg/G"));
    assert!(sequences.contains(&"/"));
    assert!(!rows.iter().any(|row| row.description.contains("cell")));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib help::tests::footer -- --nocapture`

Expected: FAIL because priorities and context rows are incomplete.

**Step 3: Populate the default footer metadata**

Implement the approved high-frequency sets. Keep labels concise enough for an
80-column terminal. Use actual behavior names, including separate Explorer `/`
find and `f` catalog search actions.

Make dynamic capabilities explicit rather than inspecting unrelated labels. At
minimum account for:

- Data Query availability.
- Relation edit availability and current relation edit mode.
- Relation tabs not having an Editor pane.
- Empty grids disabling Record View.

**Step 4: Run focused tests**

Run: `cargo test --lib help::tests::footer -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/help.rs
git commit -m "feat(ui): define contextual footer shortcuts"
```

### Task 4: Expose Validated Pending Sequence Snapshots

**Files:**
- Modify: `src/input/keymap.rs:16-50, 726-838`
- Test: `src/input/keymap.rs` test module
- Test: `tests/keymap.rs`

**Step 1: Write failing snapshot lifecycle tests**

Introduce tests around a public display-facing type:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeySequenceState {
    pub prefix: ShortcutPrefix,
    pub display: String,
    pub remaining: Duration,
}
```

Test that:

- `<space>` produces `ShortcutPrefix::Leader` outside Editor input modes.
- `<ctrl+w>` produces `ShortcutPrefix::Window` outside Editor input modes.
- `g`, `z`, `[`, `]`, relation `d`, relation `y`, and Record View `g` expose
  their corresponding semantic prefixes.
- Editor Insert `Ctrl-w` returns an editor action and no window prefix.
- A snapshot disappears after 750 ms.
- Focus, editor mode, or active tab changes invalidate it.
- A counted window sequence includes its count only after `<ctrl+w>` is entered.

Use an injectable timestamp or a `sequence_state_at(app, now)` helper so tests
do not sleep.

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib input::keymap::tests -- --nocapture`

Expected: FAIL because no public sequence snapshot exists.

**Step 3: Implement the snapshot API**

Map private `Pending` variants to public `ShortcutPrefix` values. Keep private
timing and validity details in `Keymap`. Prefer:

```rust
pub fn sequence_state(&self, app: &App, now: Instant) -> Option<KeySequenceState>;
pub fn expire_pending(&mut self, app: &App, now: Instant) -> bool;
```

`expire_pending` returns `true` only when a previously visible pending sequence
is cleared. Reuse one validity helper for `map`, `sequence_state`, and
`expire_pending` so the timeout contract cannot diverge.

Do not change command mappings or the 750 ms timeout.

**Step 4: Run keymap tests**

Run: `cargo test --lib input::keymap::tests -- --nocapture`

Run: `cargo test --test keymap -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/input/keymap.rs tests/keymap.rs
git commit -m "feat(input): expose pending key sequence hints"
```

### Task 5: Resolve Prefix Candidates from the Catalog

**Files:**
- Modify: `src/help.rs`
- Test: `src/help.rs` test module

**Step 1: Write failing candidate tests**

Add exact-set tests for all prefixes:

```rust
#[test]
fn editor_leader_candidates_include_editor_commands() {
    let candidates = prefix_shortcuts(
        ShortcutContext::EditorNormal,
        ShortcutPrefix::Leader,
        ShortcutCapabilities::default(),
    );

    assert_has_suffixes(&candidates, &["n", "s", "r", "R", "f", "d", "q", "x", "e", "y", "Y"]);
}
```

Also verify:

- Results-only `<space> Y` appears only where grid row headers can be copied.
- Window directions differ for Explorer, Editor, Results, and Relation Results.
- `g` descriptions say first node, first row, or first field as appropriate.
- `z` is available only for Explorer and grid contexts.
- Relation `d d` and `y y` appear only in browse/edit-capable relation data.
- Unknown or invalid prefix/context pairs return no candidates.

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib help::tests::prefix -- --nocapture`

Expected: FAIL because candidate filtering is not implemented.

**Step 3: Implement prefix candidate filtering**

Add a pure function that accepts the display context, capabilities, and semantic
prefix. Return rows sorted first by footer priority and then declaration order.
The function must not inspect `Keymap::Pending`.

For a counted Window prefix, preserve the count in the sequence badge supplied
by `KeySequenceState`; do not duplicate the count on every catalog row.

**Step 4: Run focused tests**

Run: `cargo test --lib help::tests::prefix -- --nocapture`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/help.rs
git commit -m "feat(ui): resolve key prefix continuations"
```

### Task 6: Build a Width-Aware Footer Hint Layout

**Files:**
- Modify: `src/ui/mod.rs:2075-2170`
- Test: `src/ui/mod.rs` unit tests or `tests/ui_render.rs`

**Step 1: Write failing pure layout tests**

Extract a small pure helper that packs complete hint units into a given terminal
cell width. Test:

```rust
#[test]
fn hint_layout_never_splits_an_entry() {
    let hints = ["j/k move", "Enter open", "/ find"];
    assert_eq!(pack_hints(&hints, 19), "j/k move   ... (+2)");
}
```

Use actual expected widths after accounting for separators and the omission
marker. Add tests for:

- All entries fitting.
- One or more entries omitted.
- Omission marker forcing the last otherwise-fitting hint out.
- No candidate fitting.
- Unicode labels measured in terminal cells, not bytes.
- Accurate omitted counts.

Prefer the cell-width helper already used in the UI instead of introducing a
second Unicode-width dependency.

**Step 2: Run tests to verify they fail**

Run: `cargo test --test ui_render footer_hint_layout -- --nocapture`

Expected: FAIL because the width-aware helper does not exist.

**Step 3: Implement the packing helper**

Pack hints as indivisible units separated by three spaces. Reserve room for
`... (+N)` before deciding that the final visible item fits. Return structured
spans if that makes key styling possible without reparsing strings; otherwise
return one string and keep styling uniform for this iteration.

**Step 4: Replace hard-coded footer strings**

Change `render_footer` to receive an optional `&KeySequenceState` and:

- Resolve the active shortcut context and capabilities.
- Use default `footer_shortcuts` when no sequence is pending.
- Use `prefix_shortcuts` when a sequence is pending.
- Include the semantic prefix label for pending candidates.
- Pack the resulting units into the width remaining after the mode badge.
- Remove the duplicate hard-coded `?/F1 help` span.
- Preserve the complete second-row status behavior unchanged.

**Step 5: Add rendering tests at representative widths**

Use `TestBackend` at 56, 80, and 120 columns. Assert that:

- Explorer shows navigation/search hints, not editor or cell hints.
- SQL Output does not say `cells`.
- Relation DDL describes text navigation.
- Narrow output includes `... (+N)` and no partial key label.

**Step 6: Run UI tests**

Run: `cargo test --test ui_render footer -- --nocapture`

Expected: PASS.

**Step 7: Commit**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(ui): render adaptive contextual key hints"
```

### Task 7: Pass Pending Sequences Through the Render Boundary

**Files:**
- Modify: `src/ui/mod.rs:252-373`
- Modify: `src/runtime.rs:2519-2598`
- Test: `src/runtime.rs` unit tests, or a small extracted event-loop state helper test
- Test: `tests/ui_render.rs`

**Step 1: Write failing redraw decision tests**

Extract the smallest pure comparison/helper needed to test these transitions:

- No sequence to Leader means redraw.
- Leader to no sequence on completion means redraw even if an Action also
  redraws.
- Leader to no sequence on timeout means redraw once.
- No sequence remaining no sequence does not redraw.
- Clearing pending with mouse/paste/focus events redraws only if a sequence was
  visible.

Avoid an end-to-end test that sleeps for 750 ms.

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib runtime::tests::key_sequence -- --nocapture`

Expected: FAIL because runtime rendering does not observe sequence state.

**Step 3: Extend UI render entry points**

Add a render entry point accepting `Option<&KeySequenceState>`. Keep existing
public convenience functions delegating with `None` so unrelated tests and
callers do not need synthetic keymaps.

For example:

```rust
pub fn render_with_input_state(
    frame: &mut Frame<'_>,
    app: &App,
    state: &mut UiState,
    icons: IconSet,
    sequence: Option<&KeySequenceState>,
)
```

Choose a concise name consistent with existing render functions.

**Step 4: Update the runtime event loop**

Before and after each key mapping, compare visible sequence snapshots and set
`redraw` when they differ. In the ticker branch, call `expire_pending` and OR its
result into `redraw` alongside clipboard and animation updates.

Pass the current snapshot to every runtime draw, including the initial draw.
Use a single `now` value per observation to prevent boundary inconsistencies.

Ensure mouse, paste, resize, and focus events set redraw if clearing pending
changed visible state, even when those events do not produce an application
Action.

**Step 5: Add an integration-style render test for a prefix**

Construct an app and keymap, enter `<space>`, obtain the snapshot, and render
through the new entry point. Assert that prefix candidates replace default
Explorer hints.

**Step 6: Run focused tests**

Run: `cargo test --lib runtime::tests::key_sequence -- --nocapture`

Run: `cargo test --test ui_render pending_prefix -- --nocapture`

Expected: PASS.

**Step 7: Commit**

```bash
git add src/runtime.rs src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(runtime): redraw pending key sequence hints"
```

### Task 8: Complete Overlay and Modal Shortcut Coverage

**Files:**
- Modify: `src/help.rs`
- Modify: `src/ui/mod.rs` only if an overlay-specific footer context needs data already available during rendering
- Test: `src/help.rs` test module
- Test: `tests/ui_render.rs`

**Step 1: Write failing coverage tests**

Add tests ensuring the display catalog covers every branch near the beginning
of `Keymap::map`, including:

- Help search.
- Profile Manager form, scope, and delete confirmation.
- Profile access.
- Record View.
- Substitute confirmation.
- Execution confirmation.
- Manual cancellation confirmation.
- Transaction exit confirmation.
- Clear transaction outcome.
- Target selector.
- Explorer find and catalog search editing/confirmed phases.
- SQL Editor List.
- Delete SQL editor confirmation.
- Data Query input and completion.

The test can maintain an explicit required-context list. Do not attempt source
code reflection.

**Step 2: Run tests to verify they fail**

Run: `cargo test --lib help::tests::modal -- --nocapture`

Expected: FAIL with missing context rows.

**Step 3: Add concise modal rows**

Populate only actual mapped controls. Text input contexts should group literal
typing as `type`, while documenting navigation and destructive controls
explicitly. Confirmation contexts should show their valid choices and cancel
key.

If several confirmations have materially different choices, model them as
distinct contexts rather than a generic `Confirmation` that advertises invalid
keys.

**Step 4: Add representative UI tests**

Render Profile Manager, Help, Record View, and one transaction confirmation.
Assert each footer shows modal controls and does not show underlying pane hints.

**Step 5: Run focused tests**

Run: `cargo test --lib help::tests::modal -- --nocapture`

Run: `cargo test --test ui_render modal_footer -- --nocapture`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/help.rs src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(help): cover modal keyboard contexts"
```

### Task 9: Rewrite the Dedicated Keyboard Reference

**Files:**
- Modify: `docs/keybindings.md`
- Modify: `README.md:266-282, 318-326`
- Test: add a documentation contract test to `tests/docs.rs` if a docs test file already exists; otherwise add a small test in the most appropriate existing integration test module

**Step 1: Write a failing documentation contract test**

Read the Markdown at compile time with `include_str!` and assert:

```rust
#[test]
fn keyboard_reference_is_dedicated_and_complete() {
    let readme = include_str!("../README.md");
    let keys = include_str!("../docs/keybindings.md");

    assert!(!readme.contains("## Essential Keys"));
    assert!(readme.contains("docs/keybindings.md"));
    for heading in [
        "## Global",
        "## Pane Navigation and Resize",
        "## Prefixes",
        "## Explorer",
        "## SQL Editor",
        "## SQL Results Data",
        "## SQL Output",
        "## Relation Data",
        "## Relation DDL",
        "## Record View",
        "## Data Query Inputs",
        "## Profile Manager",
        "## SQL Editor List",
        "## Help Search",
        "## Confirmation Dialogs",
        "## Mouse",
    ] {
        assert!(keys.contains(heading), "missing {heading}");
    }
}
```

The README assertion should target the removed table/heading rather than ban
all inline code, because unrelated installation commands legitimately remain.

**Step 2: Run the documentation test to verify it fails**

Run the exact test target selected in Step 1, for example:

`cargo test --test docs keyboard_reference_is_dedicated_and_complete -- --nocapture`

Expected: FAIL because README still contains Essential Keys and the reference
lacks the new section structure.

**Step 3: Replace README shortcut content with a reference**

Remove the Essential Keys table and concrete keyboard examples. Add a short
paragraph stating that the footer shows contextual controls and link to
`docs/keybindings.md`. Keep the Documentation table link.

**Step 4: Rewrite `docs/keybindings.md` against the real mapper**

Use the approved section order. Verify every branch in `Keymap::map` and helper
mappers rather than copying the old help list. Correct known stale statements,
especially Relation Data editing capabilities and the distinction between
Explorer `/` find and `f` catalog search.

Document:

- Key notation and the 750 ms adjacent-key timeout.
- All leader/window/goto/alignment/tab/relation prefixes.
- Context and mode restrictions.
- Dynamic no-op conditions.
- Overlay and confirmation controls.
- Aliases and mouse behavior.

Do not claim that footer truncation removes functionality; direct input and F1
help remain available.

**Step 5: Run the documentation test**

Run: `cargo test --test docs keyboard_reference_is_dedicated_and_complete -- --nocapture`

Expected: PASS.

**Step 6: Manually compare docs with keymap branches**

Run searches for mapper entry points and pending mappings:

```bash
rg "fn map_|Pending::|KeyCode::" src/input/keymap.rs
```

Expected: Every user-facing branch is represented under an appropriate
`docs/keybindings.md` section. Do not modify behavior to make it match stale
documentation; update documentation to match actual behavior.

**Step 7: Commit**

```bash
git add README.md docs/keybindings.md tests/docs.rs
git commit -m "docs: centralize keyboard reference"
```

Adjust the staged test path if Step 1 used an existing test module.

### Task 10: Run Full Verification and Fix Only Related Regressions

**Files:**
- Modify: only files changed by Tasks 1-9 when required by verification

**Step 1: Format**

Run: `cargo fmt --check`

Expected: PASS. If it fails, run `cargo fmt`, inspect the resulting diff, and
rerun `cargo fmt --check`.

**Step 2: Run focused input and UI suites**

Run: `cargo test --lib help::tests input::keymap::tests -- --nocapture`

Run: `cargo test --test keymap -- --nocapture`

Run: `cargo test --test ui_render -- --nocapture`

Expected: PASS.

**Step 3: Run Clippy**

Run: `cargo clippy --all-targets -- -D warnings`

Expected: PASS with zero warnings.

**Step 4: Run the complete suite**

Run: `cargo test`

Expected: PASS.

**Step 5: Inspect the final diff**

Run: `git status --short`

Run: `git diff --check`

Run: `git diff --stat`

Expected: Only shortcut catalog, keymap snapshot, runtime/UI plumbing, tests,
README, and keyboard documentation changes are present; no whitespace errors.
Do not revert unrelated pre-existing worktree changes.

**Step 6: Commit verification fixes if needed**

If verification required related fixes not included in an earlier commit:

```bash
git add <only-related-files>
git commit -m "fix(ui): stabilize contextual key hints"
```

If no fixes were needed, do not create an empty commit.
