# Compact Workspace Status Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the redundant header status row and footer `Ready` row, reclaim two terminal rows for workspace content, and keep meaningful SQL and relation state in the panels that own it.

**Architecture:** Keep `ConnectionStatus`, `QueryStatus`, and all reducer/runtime transitions unchanged; this is a presentation-only refactor. Collapse the global header and footer to one row each, render only non-idle SQL query state in the SQL Editor top border, and move the Relation DDL provenance/viewport context from the global footer into the Relation DDL top border. Preserve transaction visibility as the highest-priority SQL Editor runtime context at narrow widths.

**Tech Stack:** Rust 1.94, Ratatui 0.30.2, Crossterm 0.29, existing `AppLayout` geometry and `TestBackend` rendering tests.

---

## Scope and Invariants

- Do not change `QueryStatus`, `ConnectionStatus`, query execution, cancellation, failure, or completion transitions.
- Do not add a configuration flag for the old two-row header/footer; there is no persisted or external compatibility requirement.
- Do not show `QUERY IDLE` anywhere. Idle is the default and should be visually silent.
- Show `QUERY RUNNING`, `QUERY CANCELLED`, and `QUERY ERROR` only on SQL Console tabs and only in the SQL Editor top border.
- Keep the SQL Editor title and editor mode on the left.
- Keep transaction state visible at 56, 80, and 120 columns. When horizontal space is constrained, preserve runtime context in this order: transaction state, non-idle query status, execution target.
- Remove the global `ONLINE` label. Explorer profile markers, disconnected workspace rendering, notifications, and the SQL target's existing `OFFLINE` suffix remain the connection-state affordances.
- Keep the footer mode badge, contextual shortcut hints, and footer help hit region.
- Preserve Relation DDL source, viewport position, and snapshot provenance; move them into the Relation DDL panel rather than dropping them.
- Keep Relation Data's existing in-panel `SQL: ... rows ... Snapshot: ...` line unchanged.
- Use ASCII text for all new labels.
- Commit commands below are optional checkpoints and must only be run when the user has explicitly authorized commits.

## Target Rendering

Idle SQL Console:

```text
 LAZYDB   lssc-uat  /  moss_biz
╭─ SQL EDITOR  NORMAL ───────────── [lssc-uat] moss_biz.tools  TX AUTO ─╮
...
 EXPLORE   j move selection down   k move selection up   ...
```

Running SQL Console:

```text
╭─ SQL EDITOR  NORMAL ── QUERY RUNNING  [lssc-uat] moss_biz.tools  TX AUTO ─╮
```

Ready Relation DDL view:

```text
╭─ RELATION DDL ───────── NATIVE CATALOG  ROW 1  COL 1  LIVE ─╮
```

The exact border fill depends on Ratatui and terminal width. Tests should assert labels, placement, color, and retained information rather than an entire snapshot string.

## Task 1: Lock Down the One-Row Layout Contract

**Files:**
- Modify: `src/ui/layout.rs:5-11` (layout constants)
- Modify: `src/ui/layout.rs:60-70` (`AppLayout::calculate` root vertical split)
- Test: `src/ui/layout.rs:220-388`

**Step 1: Introduce named header and footer height constants**

Add constants beside the existing layout constants so the row allocation is explicit and testable:

```rust
const HEADER_HEIGHT: u16 = 1;
const FOOTER_HEIGHT: u16 = 1;
```

Do not change the split yet.

**Step 2: Add a failing geometry test**

Add this focused test to `src/ui/layout.rs`:

```rust
#[test]
fn header_and_footer_each_use_one_row() {
    let area = Rect::new(0, 0, 120, 36);
    let layout = AppLayout::calculate(
        area,
        Focus::Editor,
        false,
        PaneSizePreferences::default(),
        false,
    );

    assert_eq!(layout.header.height, 1);
    assert_eq!(layout.footer.height, 1);
    assert_eq!(layout.header.y, area.y);
    assert_eq!(layout.body.y, area.y + 1);
    assert_eq!(layout.footer.y, area.bottom() - 1);
    assert_eq!(
        layout.header.height + layout.body.height + layout.footer.height,
        area.height
    );
}
```

**Step 3: Run the test and verify the expected failure**

Run:

```bash
cargo test ui::layout::tests::header_and_footer_each_use_one_row -- --nocapture
```

Expected: FAIL because the current header and footer heights are both `2`.

**Step 4: Collapse the root layout**

Update the root vertical constraints in `AppLayout::calculate`:

```rust
let vertical = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(8),
        Constraint::Length(FOOTER_HEIGHT),
    ])
    .split(area);
```

No other pane split should change. The two reclaimed rows naturally become part of `body`; existing percentage/default calculations may therefore produce slightly taller editor/results panes.

**Step 5: Update affected geometry assertions**

Run all layout unit tests:

```bash
cargo test ui::layout::tests -- --nocapture
```

Expected: the new test passes. If fixed numeric assertions such as `editor.height == 15` change because `body` is two rows taller, update only the expected geometry produced by the existing split formula. Do not change `editor_height`, `split_results`, minimum panel heights, or compact-mode thresholds to preserve old pixel counts.

**Step 6: Commit checkpoint**

```bash
git add src/ui/layout.rs
git commit -m "refactor(ui): collapse workspace chrome to one-row regions"
```

## Task 2: Remove the Global Status Row and Footer Placeholder

**Files:**
- Modify: `tests/ui_render.rs:2602-2615` (`standard_layout_shows_stable_workspace_regions`)
- Modify: `tests/ui_render.rs` near the standard-layout tests (new compact chrome test)
- Modify: `src/ui/mod.rs:1218-1287` (`render_header`)
- Modify: `src/ui/mod.rs:2755-2842` (`render_footer`)

**Step 1: Replace the obsolete `Ready` assertion and add negative assertions**

Change `standard_layout_shows_stable_workspace_regions` so it validates the new contract:

```rust
#[test]
fn standard_layout_shows_stable_workspace_regions() {
    let output = render(&fixture(), 120, 36);

    assert!(output.contains("LAZYDB"));
    assert!(output.contains("orbital-lab"));
    assert!(output.contains("EXPLORER"));
    assert!(output.contains("console"));
    assert!(output.contains("SELECT"));
    assert!(output.contains("DATA"));
    assert!(output.contains("OUTPUT"));
    assert!(output.contains("Ada"));
    assert!(!output.contains("Ready"), "{output}");
    assert!(!output.contains("QUERY IDLE"), "{output}");
}
```

Add a separate test that verifies the header is physically one row and the old connection badge is absent. Use the rendered lines rather than a whole-screen golden snapshot:

```rust
#[test]
fn workspace_header_and_footer_render_without_redundant_status_rows() {
    let output = render(&fixture(), 120, 36);
    let lines = output.lines().collect::<Vec<_>>();

    assert!(lines[0].contains("LAZYDB"), "{output}");
    assert!(lines[0].contains("orbital-lab"), "{output}");
    assert!(!output.contains("ONLINE"), "{output}");
    assert!(!output.contains("QUERY IDLE"), "{output}");
    assert!(!output.contains("Ready"), "{output}");
    assert!(lines.last().unwrap().contains("NORMAL")
        || lines.last().unwrap().contains("EXPLORE")
        || lines.last().unwrap().contains("DATA"));
}
```

The final footer badge depends on focus, so accept the three valid mode badges instead of coupling this test to fixture focus.

**Step 2: Run both tests and verify failure**

Run:

```bash
cargo test --test ui_render standard_layout_shows_stable_workspace_regions -- --nocapture
cargo test --test ui_render workspace_header_and_footer_render_without_redundant_status_rows -- --nocapture
```

Expected: FAIL because the current output contains `ONLINE`, `QUERY IDLE`, and `Ready`.

**Step 3: Simplify `render_header` to one line**

Remove the `connection` and `query` mappings from `render_header`. Render a single `Line` containing only the existing brand, profile, separator, and database spans:

```rust
let line = Line::from(vec![
    Span::styled(
        " LAZYDB ",
        Style::new()
            .fg(theme.background)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ),
    Span::styled("  ", Style::new().bg(theme.surface)),
    Span::styled(
        profile,
        Style::new()
            .fg(theme.text)
            .bg(theme.surface)
            .add_modifier(Modifier::BOLD),
    ),
    Span::styled("  /  ", Style::new().fg(theme.border).bg(theme.surface)),
    Span::styled(database, Style::new().fg(theme.action).bg(theme.surface)),
]);

frame.render_widget(
    Paragraph::new(line).style(Style::new().bg(theme.surface)),
    area,
);
```

Keep the existing `HeaderProfile` hit-region calculation unchanged; its Y coordinate remains `area.y`, and its X offset remains valid because the first-line content does not change.

**Step 4: Simplify `render_footer` to one line**

Delete `relation_context`, `second_text`, `second_color`, and `second`. Render only the existing mode badge and packed shortcut hint line:

```rust
frame.render_widget(
    Paragraph::new(line).style(Style::new().bg(theme.surface)),
    area,
);
```

Do not change `footer_shortcuts`, `pack_hints`, mode badge styling, or the `HitTarget::Help` regions created by the caller.

**Step 5: Remove imports made unused by the refactor**

Run formatting/checking and remove only imports the compiler identifies as unused. `QueryStatus` must remain imported in `src/ui/mod.rs` because query animation/result rendering and Task 3 still use it. `ConnectionStatus` may become unused in this module after Header simplification; remove it from the local import list only if `cargo check` confirms that.

**Step 6: Verify focused UI tests**

Run:

```bash
cargo test --test ui_render standard_layout_shows_stable_workspace_regions
cargo test --test ui_render workspace_header_and_footer_render_without_redundant_status_rows
cargo test --test ui_render compact_layout_uses_the_focused_panel
cargo test --test ui_render tiny_terminal_gets_an_actionable_message
```

Expected: PASS. The tiny-terminal path remains unchanged because `LayoutMode::TooSmall` returns before Header/Footer rendering.

**Step 7: Commit checkpoint**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "refactor(ui): remove redundant workspace status rows"
```

## Task 3: Move Meaningful Query State into the SQL Editor Border

**Files:**
- Modify: `tests/ui_render.rs:19-30` (import `QueryStatus`)
- Modify: `tests/ui_render.rs` near `editor_context_keeps_transaction_visible_when_narrow`
- Modify: `src/ui/mod.rs:2178-2294` (`render_editor`)

**Step 1: Add failing query-state rendering tests**

Import `QueryStatus` with the existing workspace imports:

```rust
workspace::{ConnectionStatus, Focus, Overlay, QueryStatus},
```

Add a table-driven test:

```rust
#[test]
fn sql_editor_border_only_shows_non_idle_query_status() {
    let cases = [
        (QueryStatus::Idle, None),
        (QueryStatus::Running, Some("QUERY RUNNING")),
        (QueryStatus::Cancelled, Some("QUERY CANCELLED")),
        (QueryStatus::Failed, Some("QUERY ERROR")),
    ];

    for (status, expected) in cases {
        let mut app = fixture();
        app.focus = Focus::Editor;
        app.active_console_mut().query_status = status;
        let output = render(&app, 120, 36);

        if let Some(expected) = expected {
            assert!(output.contains(expected), "status={status:?}: {output}");
        } else {
            assert!(!output.contains("QUERY IDLE"), "{output}");
            assert!(!output.contains("QUERY RUNNING"), "{output}");
            assert!(!output.contains("QUERY CANCELLED"), "{output}");
            assert!(!output.contains("QUERY ERROR"), "{output}");
        }
    }
}
```

Add a placement and styling test using `render_buffer_with_icons` and `find_text_cell`:

```rust
#[test]
fn running_query_status_is_rendered_on_the_editor_top_border() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.active_console_mut().query_status = QueryStatus::Running;

    let (buffer, _) = render_buffer_with_icons(&app, 120, 36, IconSet::default());
    let (_, editor_y) = find_text_cell(&buffer, "SQL EDITOR").expect("editor title");
    let (status_x, status_y) = find_text_cell(&buffer, "QUERY RUNNING").expect("query status");

    assert_eq!(status_y, editor_y);
    assert_eq!(buffer[(status_x, status_y)].fg, Color::Rgb(101, 167, 255));
}
```

`Color::Rgb(101, 167, 255)` is the current `Theme::default().action` value from `src/ui/theme.rs:89`. Keep the assertion against the rendered buffer; do not make private theme internals public solely for this test. If the default palette changes before implementation, update this expected value to the then-current action color.

**Step 2: Extend the narrow transaction regression test**

Update `editor_context_keeps_transaction_visible_when_narrow` so it exercises the worst relevant state:

```rust
app.active_console_mut().query_status = QueryStatus::Running;
for width in [120, 80, 56] {
    let output = render(&app, width, 24);
    assert!(output.contains("TX MANUAL:ACTIVE"), "width={width}: {output}");
    assert!(output.contains("QUERY RUNNING"), "width={width}: {output}");
}
```

If 56 columns cannot accommodate the complete target plus both states, the target may be omitted or compacted there. Do not omit or abbreviate `TX MANUAL:ACTIVE` or `QUERY RUNNING`.

**Step 3: Run the new tests and verify failure**

Run:

```bash
cargo test --test ui_render sql_editor_border_only_shows_non_idle_query_status -- --nocapture
cargo test --test ui_render running_query_status_is_rendered_on_the_editor_top_border -- --nocapture
cargo test --test ui_render editor_context_keeps_transaction_visible_when_narrow -- --nocapture
```

Expected: the non-idle status and placement tests fail because query status is no longer in Header after Task 2 and is not yet rendered on the Editor border.

**Step 4: Build a styled query status span in `render_editor`**

Read the active console once and derive an optional label/color:

```rust
let query_status = app.active_console_opt().and_then(|tab| match tab.query_status {
    QueryStatus::Idle => None,
    QueryStatus::Running => Some(("QUERY RUNNING", theme.action)),
    QueryStatus::Cancelled => Some(("QUERY CANCELLED", theme.warning)),
    QueryStatus::Failed => Some(("QUERY ERROR", theme.error)),
});
```

Construct the right title as a `Line` rather than flattening every segment into `Line::raw`. Preserve semantic colors:

```rust
let mut context = Vec::new();
if let Some((label, color)) = query_status {
    context.push(Span::styled(
        format!(" {label} "),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    ));
}
context.push(Span::raw(format!(" {target} ")));
context.push(Span::raw(format!(" {transaction} ")));
```

Then pass `Line::from(context).right_aligned()` to the second `title_top` call.

**Step 5: Enforce narrow-width priority deliberately**

Do not rely on Ratatui title overlap/clipping to choose which information survives. Calculate the available top-border width after accounting for the left title (`SQL EDITOR` plus mode), border cells, and spacing. Build the right title from required to optional segments:

1. Always reserve space for `transaction`.
2. Reserve space for non-idle `query_status` when present.
3. Include `target` only when the remaining width can hold it.

Keep this sizing logic local to `render_editor` unless a second caller immediately needs it. Use the existing Unicode cell-width utilities already imported by `src/ui/mod.rs`; do not use byte length.

For `area.width` too small even for both mandatory segments, keep the full transaction text and full non-idle query text and let border decoration yield first. The minimum supported terminal width is 56, so no additional tiny-terminal behavior is needed.

**Step 6: Verify state, placement, color, and narrow rendering**

Run:

```bash
cargo test --test ui_render sql_editor_border_only_shows_non_idle_query_status
cargo test --test ui_render running_query_status_is_rendered_on_the_editor_top_border
cargo test --test ui_render editor_context_keeps_transaction_visible_when_narrow
cargo test --test ui_render compact_layout_uses_the_focused_panel
cargo test --test ui_render wide_layout_remains_readable
```

Expected: PASS. Also verify `QUERY RUNNING` appears exactly once in the 120-column rendered output so it has not accidentally remained in Header:

```rust
assert_eq!(output.matches("QUERY RUNNING").count(), 1, "{output}");
```

**Step 7: Commit checkpoint**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(ui): show active query state in editor border"
```

## Task 4: Move Relation DDL Context into Its Owning Panel

**Files:**
- Modify: `tests/ui_render.rs:9-16` (add any catalog imports needed by the fixture)
- Modify: `tests/ui_render.rs:1977-1989` and nearby relation tests
- Modify: `src/ui/relation.rs:418-478` (`render_ddl`, `render_ddl_editor`)
- Modify: `src/ui/mod.rs:2796-2837` only if any obsolete relation-footer code remains after Task 2

**Step 1: Add a reusable ready-DDL test fixture**

Create a small helper near the existing relation UI tests that:

- starts from `fixture()` so the snapshot connection matches the active connection;
- creates a valid `RelationTab` and valid `CatalogEntry`/`CatalogPage` using existing catalog constructors;
- sets `relation.ddl` to `RelationLoad::Ready(OwnedSnapshot<RelationDdl>)`;
- uses SQL with multiple lines so row/column viewport labels are meaningful;
- sets `RelationDdl.provenance` to `DdlProvenance::NativeCatalog`;
- switches `relation.view` to `RelationView::Ddl` and focuses Results.

Keep catalog object construction in the test helper; do not add production constructors solely to simplify the UI test.

**Step 2: Add failing information-preservation and placement tests**

Add:

```rust
#[test]
fn relation_ddl_context_is_rendered_on_the_panel_border() {
    let app = ready_relation_ddl_fixture();
    let (buffer, _) = render_buffer_with_icons(&app, 120, 36, IconSet::default());

    let (_, title_y) = find_text_cell(&buffer, "RELATION DDL").expect("DDL title");
    let (_, source_y) = find_text_cell(&buffer, "NATIVE CATALOG").expect("DDL source");
    let (_, snapshot_y) = find_text_cell(&buffer, "LIVE").expect("snapshot provenance");

    assert_eq!(source_y, title_y);
    assert_eq!(snapshot_y, title_y);
}
```

Also assert the one-row global footer contains no DDL diagnostics:

```rust
let output = render(&app, 120, 36);
let footer = output.lines().last().unwrap();
assert!(!footer.contains("DDL:"), "{output}");
assert!(!footer.contains("Snapshot:"), "{output}");
assert!(output.contains("ROW 1"), "{output}");
assert!(output.contains("COL 1"), "{output}");
```

Add one offline-snapshot variant by changing the active connection or snapshot identity and assert `OFFLINE SNAPSHOT` appears on the same border row. This protects the safety-relevant provenance state during the migration.

**Step 3: Run the tests and verify failure**

Run:

```bash
cargo test --test ui_render relation_ddl_context_is_rendered_on_the_panel_border -- --nocapture
cargo test --test ui_render relation_ddl_offline_snapshot_remains_visible -- --nocapture
```

Expected: FAIL because Task 2 removed the footer diagnostics and Relation DDL currently only renders the left panel title.

**Step 4: Derive the DDL panel context in `render_ddl`**

When the active DDL load has a current or previous snapshot, derive:

- source from `snapshot.value.provenance`:
  - `DdlProvenance::NativeCatalog` -> `NATIVE CATALOG`
  - `DdlProvenance::AdapterGenerated` -> `GENERATED`
- position from `tab.ddl_viewport.row_offset + 1` and `column_offset + 1`;
- snapshot provenance from `tab.provenance(RelationView::Ddl, app.connection.active_identity(), app.active_profile())` using existing `provenance_label`.

Use the same current/previous snapshot matching semantics that the removed footer used:

```rust
let snapshot = match &tab.ddl {
    RelationLoad::Ready(snapshot)
    | RelationLoad::Loading {
        previous: Some(snapshot),
        ..
    }
    | RelationLoad::Failed {
        previous: Some(snapshot),
        ..
    }
    | RelationLoad::Cancelled {
        previous: Some(snapshot),
    } => Some(snapshot),
    _ => None,
};
```

This preserves context while refreshing, after a refresh failure, and after cancellation.

**Step 5: Add the right-aligned title without changing load-state layout**

Build the existing panel block, then add a right-aligned `title_top` only when a snapshot exists:

```rust
let mut block = panel_block(" RELATION DDL ", app.focus == Focus::Results, theme);
if let Some(context) = ddl_context {
    block = block.title_top(Line::raw(format!(" {context} ")).right_aligned());
}
```

Pass that block through the existing loading/ready paths to `render_ddl_editor`. Do not move `Refreshing`, failure, cancellation, or retry controls into the border; those states already have actionable in-panel rendering.

Use cell-aware progressive omission at narrow widths:

1. Always retain snapshot provenance when it is not `LIVE`.
2. Retain DDL source.
3. Retain `ROW`/`COL` when space permits.
4. `LIVE` may be omitted first on a constrained display because the active connected context already implies it.

Keep the logic inside `render_ddl`; do not introduce a generic title-layout abstraction unless Task 3 and Task 4 genuinely share identical sizing semantics after implementation.

**Step 6: Verify Relation DDL ready and loading states**

Run:

```bash
cargo test --test ui_render relation_ddl_context_is_rendered_on_the_panel_border
cargo test --test ui_render relation_ddl_offline_snapshot_remains_visible
cargo test --test ui_render empty_relation_ddl_uses_the_ddl_panel_empty_state
cargo test --test ui_render relation_loading_with_previous_snapshot_keeps_data_visible_and_exposes_cancel
cargo test --test ui_render relation_page_renders_data_ddl_selectors_and_relation_layout
```

Expected: PASS. Empty DDL still shows `No DDL available`; ready and previous-snapshot DDL preserve source/provenance context; Relation Data behavior is unchanged.

**Step 7: Commit checkpoint**

```bash
git add src/ui/relation.rs tests/ui_render.rs
git commit -m "refactor(ui): move DDL context into relation panel"
```

## Task 5: Run Formatting and Regression Verification

**Files:**
- Verify: `src/ui/layout.rs`
- Verify: `src/ui/mod.rs`
- Verify: `src/ui/relation.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Format the changed Rust files**

Run:

```bash
cargo fmt --all -- --check
```

Expected: PASS. If it fails, run `cargo fmt --all`, inspect the resulting diff, then rerun the check.

**Step 2: Run the complete UI render suite**

Run:

```bash
cargo test --test ui_render -- --nocapture
```

Expected: PASS. Pay particular attention to tests that inspect exact row positions, popup bounds, cursor positions, and compact layouts because the workspace body starts one row earlier and ends one row later.

**Step 3: Run all UI module unit tests**

Run:

```bash
cargo test ui:: -- --nocapture
```

Expected: PASS.

**Step 4: Run static analysis**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS with no unused imports, needless allocation warnings, or style regressions.

**Step 5: Run the complete test suite**

Run:

```bash
cargo test --all-targets --all-features
```

Expected: PASS.

**Step 6: Perform a manual terminal acceptance check**

Run the application through the project's normal development command and verify at 120x36, 80x24, and the minimum supported 56x16:

- Header is exactly one row and contains brand/profile/database only.
- No global `ONLINE`, `QUERY IDLE`, or `Ready` appears.
- Footer is exactly one row and retains mode plus contextual key hints.
- SQL Editor idle state is silent.
- Running, failed, and cancelled SQL states appear on the SQL Editor top border with appropriate semantic colors.
- `TX MANUAL:ACTIVE`, `TX ABORTED`, and `TX UNKNOWN` remain visible at narrow widths.
- Explorer, Editor, Results, Relation, and Dashboard layouts all gain usable vertical space without overlap.
- Relation DDL source and offline/out-of-scope/deleted-profile snapshot provenance remain visible in the DDL panel.
- Mouse help targeting still covers the one-row footer.

Record any terminal/font-specific border overlap as a rendering bug and fix it before completion; do not weaken tests by merely removing narrow-width assertions.

**Step 7: Inspect the final diff**

Run:

```bash
git diff --check
git diff -- src/ui/layout.rs src/ui/mod.rs src/ui/relation.rs tests/ui_render.rs
```

Expected: no whitespace errors and no changes to application state transitions, persistence formats, public APIs, or database execution paths.

**Step 8: Final commit checkpoint**

```bash
git add src/ui/layout.rs src/ui/mod.rs src/ui/relation.rs tests/ui_render.rs
git commit -m "feat(ui): compact workspace status chrome"
```

Skip this commit if the preceding tasks were already committed individually and there are no remaining changes.

## Acceptance Criteria

- Global Header consumes exactly one terminal row.
- Global Footer consumes exactly one terminal row.
- Workspace body gains two terminal rows at every supported non-tiny size.
- `ONLINE`, `QUERY IDLE`, and default `Ready` no longer appear in normal workspace rendering.
- `QueryStatus` remains unchanged as application state and continues driving execution guards and loading animations.
- SQL Editor displays `QUERY RUNNING`, `QUERY CANCELLED`, and `QUERY ERROR`; it displays nothing for Idle.
- SQL query state is on the same row as the SQL Editor top border and uses action/warning/error colors.
- Transaction state remains visible at 56, 80, and 120 columns.
- Relation DDL source, row/column position, and safety-relevant snapshot provenance are not lost.
- Relation Data's existing SQL/row/snapshot context remains unchanged.
- Contextual footer hints and footer Help mouse target continue to work.
- Layout unit tests, full UI render tests, Clippy, and the complete test suite pass.
