# SQL Editor Completion Anchor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Anchor SQL Editor completion candidate names to the first displayed cell of the identifier being replaced, so continued typing changes the candidates without moving the popup.

**Architecture:** Keep completion matching and replacement ranges unchanged. Convert the first candidate's existing `CompletionCandidate::replace.start` byte offset into a visible editor cell through `EditorRenderSnapshot::lines[*].source_to_display_cells`, then let the popup renderer subtract one fixed-width icon column before calculating the popup rectangle. Preserve the preferred left edge near the right boundary by shrinking the popup width instead of shifting the popup left; Relation Query completion continues to use its cursor anchor.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm 0.29, `unicode-width`, existing SQL completion and editor snapshot models.

---

## Scope And Invariants

- Modify only `src/ui/mod.rs` and `tests/ui_render.rs` unless implementation reveals a concrete compile-time need elsewhere.
- Do not change `sql::complete`, `identifier_at`, `CompletionCandidate`, or `CompletionPopup`.
- Treat `CompletionCandidate::replace.start` as the semantic start of the active identifier. All candidates produced by one `sql::complete` call already share this range.
- Use UTF-8 byte offsets only for slicing source text. Convert to a character column before indexing `source_to_display_cells`.
- Use terminal cell widths, not bytes or Unicode scalar counts, for all horizontal layout.
- Keep popup vertical placement based on the editor cursor row.
- Keep the candidate type icon. Candidate label first letters, not popup borders or icons, align with the identifier first letter.
- Keep Relation Query completion cursor-anchored and behaviorally unchanged.
- When the preferred popup origin is close to the right edge, keep `x` stable and reduce `width`; do not shift `x` left as candidate content changes.
- When the identifier start is horizontally scrolled out of view, clamp its label anchor to the text viewport's left edge.
- When there is insufficient room to the left for the icon column, clamp the popup to the editor inner boundary and accept that exact label alignment is impossible at that boundary.

### Task 1: Lock Down Stable Popup Rectangle Semantics

**Files:**
- Modify: `src/ui/mod.rs:515-550`
- Test: `src/ui/mod.rs:2373-2433`

**Step 1: Replace the right-edge unit test with the required stable-origin behavior**

Rename `clamps_popup_to_the_text_viewport` to `keeps_popup_origin_and_shrinks_width_at_the_right_edge`. Keep the same viewport and preferred origin, but expect the popup to remain at `x = 48` and use the two remaining columns:

```rust
#[test]
fn keeps_popup_origin_and_shrinks_width_at_the_right_edge() {
    let anchor = CompletionAnchor {
        viewport: Rect::new(10, 5, 40, 10),
        cursor: Position::new(48, 6),
        replacement_start_x: None,
    };

    assert_eq!(
        completion_popup_rect(anchor, 20, 4),
        Some(Rect::new(48, 7, 2, 4))
    );
}
```

Add `replacement_start_x: None` to the two existing below/above test anchors so they compile after Task 2 introduces that field.

Add a left-bound test:

```rust
#[test]
fn clamps_popup_origin_to_the_viewport_left_edge() {
    let anchor = CompletionAnchor {
        viewport: Rect::new(10, 5, 40, 10),
        cursor: Position::new(4, 6),
        replacement_start_x: None,
    };

    assert_eq!(
        completion_popup_rect(anchor, 20, 4),
        Some(Rect::new(10, 7, 20, 4))
    );
}
```

**Step 2: Run the focused unit tests and verify the new right-edge assertion fails**

Run:

```bash
cargo test ui::completion_popup_tests --lib
```

Expected: compilation may temporarily fail because `replacement_start_x` is not defined yet. If the field is deferred to Task 2, first omit it from these literals; after compilation, `keeps_popup_origin_and_shrinks_width_at_the_right_edge` must fail with the current result `Rect { x: 30, width: 20, ... }`.

**Step 3: Change horizontal rectangle calculation to preserve the preferred origin**

In `completion_popup_rect`, replace the current width-first/x-shift logic:

```rust
let width = desired_width.min(viewport.width).max(1);
let x = anchor
    .cursor
    .x
    .clamp(viewport.x, viewport.right().saturating_sub(1))
    .min(viewport.right().saturating_sub(width));
```

with origin-first/width-shrink logic:

```rust
let x = anchor
    .cursor
    .x
    .clamp(viewport.x, viewport.right().saturating_sub(1));
let width = desired_width
    .min(viewport.right().saturating_sub(x))
    .max(1);
```

Do not change the vertical placement branches.

**Step 4: Run the focused unit tests**

Run:

```bash
cargo test ui::completion_popup_tests --lib
```

Expected: all completion popup rectangle tests pass.

**Step 5: Commit the rectangle behavior independently**

```bash
git add src/ui/mod.rs
git commit -m "fix(ui): keep completion popup origin stable"
```

Only commit when explicitly requested by the user; otherwise leave the verified change uncommitted.

### Task 2: Convert Completion Replacement Start To A Screen Cell

**Files:**
- Modify: `src/ui/mod.rs:357-361`
- Modify: `src/ui/mod.rs:1279-1321`
- Test: `src/ui/mod.rs:2373-2433`

**Step 1: Add focused unit tests for document-offset conversion**

Introduce a private helper named `completion_replacement_start_cell` near `render_editor`. Test it in `completion_popup_tests` using a small snapshot fixture or a narrower extracted helper that accepts the current `EditorRenderLine`, logical line number, horizontal offset, viewport width, source text, and replacement byte offset.

Preferred production signature:

```rust
fn completion_replacement_start_cell(
    text: &str,
    snapshot: &EditorRenderSnapshot,
    replace: crate::sql::TextRange,
) -> Option<u16>
```

The tests must cover these exact semantics:

```rust
// ASCII: "SELECT * FROM sys_u" maps replacement start to the `s` cell.
assert_eq!(start_cell, Some(14));

// Wide characters before the identifier use display cells, not char or byte count.
// "界🙂 sys_u": 2 + 2 + 1 cells before `s`.
assert_eq!(start_cell, Some(5));

// A tab uses the existing four-cell tab-stop projection.
// "\tsys_u": `s` begins at cell 4.
assert_eq!(start_cell, Some(4));

// Horizontal scrolling subtracts the snapshot offset.
// A start at display cell 14 with horizontal_offset 10 appears at cell 4.
assert_eq!(start_cell, Some(4));

// A replacement start left of horizontal_offset clamps to visible cell zero.
assert_eq!(start_cell, Some(0));

// A non-boundary or out-of-line byte offset returns None instead of panicking.
assert_eq!(start_cell, None);
```

If constructing a full `EditorRenderSnapshot` makes the tests noisy, extract one additional pure helper:

```rust
fn source_byte_to_visible_cell(
    source: &str,
    source_to_display_cells: &[usize],
    byte: usize,
    horizontal_offset: usize,
    viewport_width: usize,
) -> Option<u16>
```

Keep `completion_replacement_start_cell` responsible for locating the current document line and delegating to this helper. Do not duplicate `project_editor_line` in UI code.

**Step 2: Run the new conversion tests and verify they fail**

Run:

```bash
cargo test ui::completion_popup_tests --lib
```

Expected: FAIL because the conversion helper does not exist or returns the wrong cell for Unicode, tabs, or horizontal scrolling.

**Step 3: Implement byte-offset-to-screen-cell conversion**

Implement the helper with the following sequence:

```rust
fn completion_replacement_start_cell(
    text: &str,
    snapshot: &EditorRenderSnapshot,
    replace: crate::sql::TextRange,
) -> Option<u16> {
    if replace.start > text.len() || !text.is_char_boundary(replace.start) {
        return None;
    }

    let line_start = if snapshot.cursor.line == 0 {
        0
    } else {
        text.match_indices('\n')
            .nth(snapshot.cursor.line.saturating_sub(1))
            .map(|(offset, _)| offset + 1)?
    };
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |offset| line_start + offset);
    if replace.start < line_start || replace.start > line_end {
        return None;
    }

    let column = text[line_start..replace.start].chars().count();
    let line = snapshot
        .lines
        .iter()
        .find(|line| line.line == snapshot.cursor.line)?;
    let cell = *line.source_to_display_cells.get(column)?;
    let visible = cell.saturating_sub(snapshot.horizontal_offset);
    Some(visible.min(snapshot.viewport.width.saturating_sub(1)) as u16)
}
```

Implementation notes:

- Import `EditorRenderSnapshot` from the existing editor model import list rather than using a fully qualified type if that matches local style.
- `replace.start` is a document byte offset; never index `source_to_display_cells` with it directly.
- `snapshot.cursor.line` is valid here because SQL identifier completion cannot span a newline and `replace.end` is the cursor offset.
- `saturating_sub` deliberately clamps an off-screen-left identifier start to the visible left edge.
- Return `None` for invalid or stale completion ranges; callers must fall back to cursor anchoring.

**Step 4: Extend `CompletionAnchor` with an optional semantic label anchor**

Change:

```rust
pub(crate) struct CompletionAnchor {
    pub(crate) viewport: Rect,
    pub(crate) cursor: Position,
}
```

to:

```rust
pub(crate) struct CompletionAnchor {
    pub(crate) viewport: Rect,
    pub(crate) cursor: Position,
    pub(crate) replacement_start_x: Option<u16>,
}
```

Semantics:

- `cursor.y` remains the input row used for above/below placement.
- `cursor.x` remains the fallback popup origin.
- `replacement_start_x` is the absolute screen column where the candidate label must begin.
- Relation Query anchors set `replacement_start_x: None`.

Update every constructor:

- SQL Editor constructor near `src/ui/mod.rs:1315`.
- Result Query constructor near `src/ui/mod.rs:1597`.
- All unit-test constructors near `src/ui/mod.rs:2397-2426`.

**Step 5: Populate the semantic anchor in `render_editor`**

Before constructing `CompletionAnchor`, read the active popup replacement range and editor text:

```rust
let replacement_start_x = app
    .active_console_opt()
    .and_then(|tab| tab.completion.as_ref())
    .and_then(|popup| popup.candidates.first())
    .and_then(|candidate| {
        let text = app.active_editor_text().ok()?;
        completion_replacement_start_cell(&text, &snapshot, candidate.replace)
    })
    .map(|x| text_viewport.x.saturating_add(x));
```

Construct the SQL Editor anchor with the editor `inner` rectangle as its popup bounds:

```rust
CompletionAnchor {
    viewport: inner,
    cursor: Position::new(
        text_viewport.x.saturating_add(x),
        text_viewport.y.saturating_add(y),
    ),
    replacement_start_x,
}
```

Using `inner` rather than `text_viewport` allows the retained icon column to occupy the line-number gutter when the identifier begins at the first visible text cell. The final rectangle must still stay inside the editor panel.

For Result Query completion, preserve the current bounds and add only:

```rust
replacement_start_x: None,
```

**Step 6: Run formatting and focused unit tests**

Run:

```bash
cargo fmt -- src/ui/mod.rs
cargo test ui::completion_popup_tests --lib
```

Expected: conversion, below/above, left-bound, and right-edge tests pass.

**Step 7: Commit the coordinate conversion independently**

```bash
git add src/ui/mod.rs
git commit -m "feat(ui): derive completion anchor from replacement range"
```

Only commit when explicitly requested by the user.

### Task 3: Align Candidate Labels Behind A Fixed Icon Column

**Files:**
- Modify: `src/ui/mod.rs:363-441`
- Test: `tests/ui_render.rs:919-961`

**Step 1: Add a test helper that returns rendered terminal cells**

The existing `render_with_icons` flattens the Ratatui buffer to a string, which is not reliable for locating columns around wide characters. Add a test-only helper that clones the buffer after rendering:

```rust
fn render_buffer_with_icons(
    app: &App,
    width: u16,
    height: u16,
    icons: IconSet,
) -> (ratatui::buffer::Buffer, UiState) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state_using_icons(frame, app, &mut state, icons))
        .unwrap();
    (terminal.backend().buffer().clone(), state)
}
```

Add a helper to find a text sequence by terminal cell on one row. It should compare consecutive buffer symbols rather than byte positions in a flattened string:

```rust
fn find_cells(buffer: &ratatui::buffer::Buffer, y: u16, text: &str) -> Option<u16> {
    let width = buffer.area.width;
    (0..width).find(|start| {
        text.chars().enumerate().all(|(offset, character)| {
            start.saturating_add(offset as u16) < width
                && buffer[(start + offset as u16, y)].symbol() == character.to_string()
        })
    })
}
```

Use this helper only for ASCII needles such as `sys_u` and `sys_user`. The surrounding SQL can still contain Unicode or tabs.

**Step 2: Replace the existing cursor-anchor integration test with an identifier-label alignment test**

Rename `completion_popup_is_anchored_below_the_editor_cursor` to `completion_candidate_label_aligns_with_identifier_start`.

Build the editor in Insert mode by starting from an empty replacement and pasting the complete SQL:

```rust
let mut app = fixture();
app.focus = Focus::Editor;
app.update(Action::ReplaceEditor(String::new()));
app.update(Action::EditorKey(KeyEvent::new(
    KeyCode::Char('i'),
    KeyModifiers::NONE,
)));
app.update(Action::EditorPaste("SELECT * FROM sys_u".into()));
```

Install a deterministic table completion:

```rust
app.active_console_mut().completion = Some(CompletionPopup {
    candidates: vec![CompletionCandidate {
        label: "sys_user".into(),
        insert_text: "sys_user".into(),
        kind: CompletionKind::Table,
        detail: Some("(main)".into()),
        replace: TextRange::new(14, 19),
        score: CompletionScore {
            context: 3,
            name_match: 2,
            schema: 1,
        },
    }],
    selected: 0,
});
```

Render with `IconMode::Ascii`, find the SQL identifier on the cursor row, find `sys_user` on `state.completion_popup.unwrap().y`, and assert equal columns:

```rust
let (buffer, state) = render_buffer_with_icons(
    &app,
    120,
    36,
    IconSet::new(IconMode::Ascii),
);
let popup = state.completion_popup.unwrap();
let identifier_x = (0..popup.y)
    .find_map(|y| find_cells(&buffer, y, "sys_u"))
    .expect("SQL identifier");
let candidate_x = find_cells(&buffer, popup.y, "sys_user").expect("completion candidate");

assert_eq!(candidate_x, identifier_x);
```

Retain the existing vertical and bounds assertions:

```rust
assert_eq!(popup.y, editor.y + 2);
assert!(popup.right() <= editor.right());
assert!(popup.bottom() <= editor.bottom());
```

**Step 3: Add a mixed-kind fixed-label-column test**

Create two candidates with different completion kinds and use ASCII icons, for example `Keyword` (`KW`) and `Table` (`TB`). Assert both labels begin at the same terminal column. This locks down a single icon column even if icon strings differ in future modes.

```rust
assert_eq!(
    find_cells(&buffer, popup.y, "sys_user"),
    find_cells(&buffer, popup.y + 1, "SELECT")
);
```

Use candidate labels that are unique in the buffer to avoid matching SQL source text.

**Step 4: Run the two integration tests and verify label alignment fails**

Run:

```bash
cargo test --test ui_render completion_candidate_label_aligns_with_identifier_start -- --exact
cargo test --test ui_render completion_candidate_labels_share_a_fixed_icon_column -- --exact
```

Expected: the first candidate label begins to the right of `sys_u`; the current popup is still anchored at the cursor. The second test may already pass for same-width ASCII icons but remains a regression guard.

**Step 5: Calculate one fixed icon column width for the popup**

At the start of `render_completion_popup`, after checking candidates and before calculating `desired_width`, add:

```rust
let icon_width = popup
    .candidates
    .iter()
    .map(|candidate| icons.completion(candidate.kind).cell_width())
    .max()
    .unwrap_or(0);
let label_offset = icon_width.saturating_add(1);
```

Calculate candidate width using `label_offset` instead of formatting each individual icon:

```rust
let desired_width = popup
    .candidates
    .iter()
    .map(|candidate| {
        let detail = candidate.detail.as_deref().unwrap_or("");
        label_offset
            .saturating_add(candidate.label.as_str().cell_width())
            .saturating_add(if detail.is_empty() {
                0
            } else {
                2usize.saturating_add(detail.cell_width())
            })
            .saturating_add(1)
    })
    .max()
    .unwrap_or(4)
    .max(4);
```

Keep the existing trailing one-cell allowance unless a focused snapshot demonstrates it is unnecessary.

**Step 6: Convert the label anchor into a popup origin**

Before calling `completion_popup_rect`, derive a layout-only anchor:

```rust
let popup_x = anchor
    .replacement_start_x
    .map(|label_x| label_x.saturating_sub(label_offset.min(u16::MAX as usize) as u16))
    .unwrap_or(anchor.cursor.x);
let layout_anchor = CompletionAnchor {
    cursor: Position::new(popup_x, anchor.cursor.y),
    replacement_start_x: None,
    ..anchor
};
let Some(area) = completion_popup_rect(layout_anchor, desired_width, desired_height) else {
    return;
};
```

Do not subtract the typed prefix length. The semantic start was already converted from `replace.start`.

**Step 7: Pad every icon into the fixed-width column**

Inside the item mapping, replace:

```rust
Span::styled(format!("{} ", icons.completion(candidate.kind)), row_style)
```

with:

```rust
let icon = icons.completion(candidate.kind);
let icon_padding = " ".repeat(icon_width.saturating_sub(icon.cell_width()));
// ...
Span::styled(format!("{icon_padding}{icon} "), row_style)
```

This guarantees each candidate label starts at `area.x + label_offset`.

**Step 8: Run formatting and focused integration tests**

Run:

```bash
cargo fmt -- src/ui/mod.rs tests/ui_render.rs
cargo test --test ui_render completion_candidate_label_aligns_with_identifier_start -- --exact
cargo test --test ui_render completion_candidate_labels_share_a_fixed_icon_column -- --exact
```

Expected: both tests pass, with the ASCII `sys_user` label starting in the same cell as the SQL `sys_u` identifier.

**Step 9: Commit the label alignment independently**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "fix(ui): align completion labels with identifier start"
```

Only commit when explicitly requested by the user.

### Task 4: Prove The Popup Does Not Move While Typing

**Files:**
- Test: `tests/ui_render.rs:919-963`

**Step 1: Add a helper for a deterministic editor completion state**

Avoid repeating setup across stability tests:

```rust
fn completion_app(sql: &str, replace: TextRange, label: &str) -> App {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.update(Action::ReplaceEditor(String::new()));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorPaste(sql.into()));
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: vec![CompletionCandidate {
            label: label.into(),
            insert_text: label.into(),
            kind: CompletionKind::Table,
            detail: Some("(main)".into()),
            replace,
            score: CompletionScore {
                context: 3,
                name_match: 2,
                schema: 1,
            },
        }],
        selected: 0,
    });
    app
}
```

Keep it local to `tests/ui_render.rs`; do not add production fixtures.

**Step 2: Add the continued-typing stability test**

Render these states independently:

```rust
let cases = [
    ("SELECT * FROM s", TextRange::new(14, 15)),
    ("SELECT * FROM sy", TextRange::new(14, 16)),
    ("SELECT * FROM sys", TextRange::new(14, 17)),
    ("SELECT * FROM sys_u", TextRange::new(14, 19)),
];
```

For every state, render with ASCII icons and collect:

- `state.completion_popup.unwrap().x`
- the cell column of the candidate label
- the cell column of the typed identifier

Assert:

```rust
assert!(popup_x_values.windows(2).all(|pair| pair[0] == pair[1]));
assert!(label_x_values.windows(2).all(|pair| pair[0] == pair[1]));
assert_eq!(label_x_values, identifier_x_values);
```

**Step 3: Add a varying-candidate-width stability test near the right edge**

Use the same identifier start with short and long candidate labels in a narrow editor. Assert:

```rust
assert_eq!(short_popup.x, long_popup.x);
assert!(short_popup.width >= long_popup.width || long_popup.right() == editor.right());
assert_eq!(long_popup.right(), editor.right());
```

The exact width assertion should reflect available cells, but the required invariant is identical `x` and no overflow. This validates Task 1's width-shrink behavior through the full render path.

**Step 4: Run stability tests and verify they pass**

Run:

```bash
cargo test --test ui_render completion_popup_stays_fixed_while_typing -- --exact
cargo test --test ui_render completion_popup_keeps_origin_when_candidate_width_changes -- --exact
```

Expected: both tests pass.

**Step 5: Commit the stability regressions independently**

```bash
git add tests/ui_render.rs
git commit -m "test(ui): cover stable completion popup positioning"
```

Only commit when explicitly requested by the user.

### Task 5: Cover Display-Width And Boundary Cases

**Files:**
- Test: `src/ui/mod.rs:2373-2433`
- Test: `tests/ui_render.rs:919-1000`

**Step 1: Add a Unicode-and-tab integration case**

Use SQL where the same line contains both a tab and wide characters before the identifier:

```rust
let sql = "\tSELECT '界🙂' FROM sys_u";
let start = sql.find("sys_u").unwrap();
let app = completion_app(
    sql,
    TextRange::new(start, start + "sys_u".len()),
    "sys_user",
);
```

Render with ASCII icons and assert the candidate `sys_user` cell equals the source `sys_u` cell. Use the buffer-cell helper, not flattened string byte positions.

**Step 2: Add left-boundary fallback coverage**

Use `sys_u` at logical column zero. Assert:

- the popup remains within the editor hit region;
- the popup does not underflow;
- the candidate remains visible;
- exact label alignment may be relaxed only if the icon column cannot fit inside the editor inner bounds.

Do not remove or truncate the icon solely to satisfy left-edge alignment.

**Step 3: Add horizontal-scroll conversion coverage**

Prefer the pure helper unit test from Task 2 for deterministic horizontal offsets. If an existing public action can scroll the SQL Editor horizontally without coupling the test to keymap details, add one integration assertion as well:

- identifier start visible: align exactly;
- identifier start left of viewport: clamp the semantic label anchor to the text viewport left edge;
- popup stays inside editor bounds.

Do not introduce a production API only to drive this test.

**Step 4: Add icon-mode regression coverage**

Render the standard `sys_u` case with:

```rust
for mode in [IconMode::NerdFont, IconMode::Unicode, IconMode::Ascii] {
    // candidate label cell equals source identifier cell
}
```

If `TestBackend` represents a configured Nerd Font glyph with an environment-dependent width, assert bounds and fixed candidate label columns for that mode, while retaining strict source-label equality for Unicode and ASCII. The production calculation must still use `.cell_width()` for every mode.

**Step 5: Verify Relation Query completion did not move regressively**

Run the existing test:

```bash
cargo test --test ui_render relation_query_completion_is_anchored_to_active_input -- --exact
```

Expected: PASS. Its `CompletionAnchor` has `replacement_start_x: None`, so it remains cursor-anchored.

**Step 6: Run all focused UI tests**

Run:

```bash
cargo test --test ui_render completion -- --nocapture
cargo test ui::completion_popup_tests --lib
```

Expected: all SQL Editor and Relation Query completion rendering tests pass.

**Step 7: Commit boundary regression coverage independently**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "test(ui): cover completion anchor display widths"
```

Only commit when explicitly requested by the user.

### Task 6: Final Regression Verification

**Files:**
- Verify: `src/ui/mod.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Format the changed Rust files**

Run:

```bash
cargo fmt -- src/ui/mod.rs tests/ui_render.rs
```

Expected: command exits successfully and changes no unrelated files.

**Step 2: Run formatting check**

Run:

```bash
cargo fmt -- --check
```

Expected: PASS.

**Step 3: Run focused completion logic tests**

Run:

```bash
cargo test --test sql_completion
```

Expected: PASS. This confirms the unchanged completion range and candidate semantics still hold.

**Step 4: Run focused UI rendering tests**

Run:

```bash
cargo test --test ui_render
```

Expected: PASS, including SQL Editor completion, Relation Query completion, icon mode, hostile input projection, and editor display-cell tests.

**Step 5: Run library tests**

Run:

```bash
cargo test --lib
```

Expected: PASS.

**Step 6: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS. If an existing unrelated warning fails the command, record it verbatim and separately run Clippy on the directly affected target where feasible; do not suppress unrelated warnings.

**Step 7: Run the full test suite**

Run:

```bash
cargo test --all-targets
```

Expected: PASS.

**Step 8: Inspect the final diff**

Run:

```bash
git diff -- src/ui/mod.rs tests/ui_render.rs
git status --short
```

Confirm:

- no SQL completion matching code changed;
- no completion state model changed;
- SQL Editor uses `replace.start` for horizontal semantic anchoring;
- candidate labels share one fixed icon column;
- popup width shrinks rather than moving left near the right boundary;
- Relation Query completion still uses cursor anchoring;
- no unrelated user changes were modified.

**Step 9: Create a final commit only when requested**

If the implementation has not been committed incrementally and the user explicitly requests a commit:

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "fix(ui): stabilize SQL completion popup position"
```

## Acceptance Checklist

- In `SELECT * FROM sys_u`, the `s` in every candidate label is in the same terminal column as the `s` in `sys_u`.
- Typing `s`, `sy`, `sys`, `sys_`, and `sys_u` keeps the popup `x` coordinate unchanged.
- Changing candidate labels or details does not move the popup left edge.
- Candidate type icons remain visible and occupy one fixed-width column.
- Multiline SQL uses the current logical line's replacement start.
- Tabs, CJK characters, emoji, and terminal-safe control projections use displayed cell widths.
- Horizontal scrolling either preserves exact alignment or clamps safely when the identifier start is off-screen.
- Popup rectangles remain inside the SQL Editor inner area.
- Above/below placement remains unchanged.
- Normal mode still suppresses the SQL Editor completion popup.
- Relation Query completion remains cursor-anchored.
- `cargo fmt -- --check`, focused tests, Clippy, and `cargo test --all-targets` pass or any pre-existing unrelated failure is reported verbatim.
