# SQL Editor Cursor Style Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the terminal cursor visibly follow SQL Editor mode, using a block in Normal mode, a bar in Insert mode and prompts, and an underline in Replace mode.

**Architecture:** Keep mode-to-style projection in `render_editor`, which records the desired style in `UiState`. Keep terminal side effects in the runtime by applying that style through `TerminalSession` immediately after every completed draw.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm 0.29, the existing integration test suite.

---

### Task 1: Cover Cursor Style Transitions

**Files:**
- Modify: `tests/ui_render.rs:535-553`

**Step 1: Extend the existing mode test**

Expand `cursor_style_follows_editor_mode` so it also sends Escape after entering Insert mode and asserts that the style returns to `CursorStyle::Block`.

```rust
app.update(Action::EditorKey(KeyEvent::new(
    KeyCode::Esc,
    KeyModifiers::NONE,
)));
let (_, returned_normal_state) = render_with_state(&app, 120, 36);
assert_eq!(
    returned_normal_state.cursor_style,
    Some(lazydb::ui::CursorStyle::Block)
);
```

**Step 2: Add a prompt cursor test**

Open the command prompt from Normal mode and assert that the prompt overrides the Normal block with a bar.

```rust
#[test]
fn editor_prompt_uses_bar_cursor() {
    let mut app = fixture();
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char(':'),
        KeyModifiers::NONE,
    )));

    let (_, state) = render_with_state(&app, 120, 36);
    assert_eq!(state.cursor_style, Some(lazydb::ui::CursorStyle::Bar));
}
```

**Step 3: Add Replace mode coverage**

Send `R` from Normal mode and assert that rendering selects `CursorStyle::Underline`.

```rust
#[test]
fn replace_mode_uses_underline_cursor() {
    let mut app = fixture();
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('R'),
        KeyModifiers::NONE,
    )));

    let (_, state) = render_with_state(&app, 120, 36);
    assert_eq!(
        state.cursor_style,
        Some(lazydb::ui::CursorStyle::Underline)
    );
}
```

**Step 4: Run the focused tests and verify the regression**

Run:

```bash
cargo test --test ui_render cursor_style -- --nocapture
cargo test --test ui_render editor_prompt_uses_bar_cursor -- --nocapture
cargo test --test ui_render replace_mode_uses_underline_cursor -- --nocapture
```

Expected before the UI fix:

- Existing Normal and Insert assertions pass.
- Escape and Replace assertions should pass if the current generic mode mapping is retained.
- The prompt assertion fails with `left: Some(Block), right: Some(Bar)`, demonstrating the missing prompt override.

### Task 2: Restore Prompt-Aware UI Projection

**Files:**
- Modify: `src/ui/mod.rs:738-742`
- Test: `tests/ui_render.rs`

**Step 1: Give an active editor prompt precedence**

Replace the direct mode match with a prompt-aware selection:

```rust
state.cursor_style = Some(if snapshot.prompt.is_some() {
    CursorStyle::Bar
} else {
    match snapshot.mode {
        EditorMode::Insert => CursorStyle::Bar,
        EditorMode::Replace => CursorStyle::Underline,
        _ => CursorStyle::Block,
    }
});
```

Do not move terminal I/O into the UI or editor modules.

**Step 2: Run the cursor-style UI tests**

Run:

```bash
cargo test --test ui_render cursor_style -- --nocapture
cargo test --test ui_render editor_prompt_uses_bar_cursor -- --nocapture
cargo test --test ui_render replace_mode_uses_underline_cursor -- --nocapture
```

Expected: all selected tests pass.

### Task 3: Apply the Projected Style to the Terminal

**Files:**
- Modify: `src/runtime.rs:1997-1999`
- Modify: `src/runtime.rs:2049-2053`
- Verify existing implementation: `src/terminal.rs:65-72`

**Step 1: Apply the style after the initial draw**

Immediately after the first call to `terminal.draw`, apply the style projected into `ui_state`:

```rust
terminal
    .draw(|frame| ui::render_with_state_using_icons(frame, &app, &mut ui_state, icons))?;
if let Some(style) = ui_state.cursor_style {
    terminal.set_cursor_style(style)?;
}
sync_editor_viewport(&mut app, &mut runtime, &ui_state);
```

This ensures the initial editor state is represented before the first input event.

**Step 2: Apply the style after every redraw**

Add the same application directly after the draw in the `redraw` branch:

```rust
terminal.draw(|frame| {
    ui::render_with_state_using_icons(frame, &app, &mut ui_state, icons)
})?;
if let Some(style) = ui_state.cursor_style {
    terminal.set_cursor_style(style)?;
}
sync_editor_viewport(&mut app, &mut runtime, &ui_state);
```

Do not cache the previously sent style in this change. Reapplying after each actual redraw is simple and protects against cursor resets caused by focus or terminal behavior.

**Step 3: Confirm cleanup behavior remains intact**

Verify that `TerminalSession::drop` still executes:

```rust
SetCursorStyle::DefaultUserShape
```

No cleanup change is required.

**Step 4: Run formatting and compile checks**

Run:

```bash
cargo fmt --check
cargo check --all-targets
```

Expected: both commands exit successfully.

### Task 4: Run Regression Verification

**Files:**
- Verify: `src/runtime.rs`
- Verify: `src/ui/mod.rs`
- Verify: `src/terminal.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Run the focused integration test file**

Run:

```bash
cargo test --test ui_render
```

Expected: all UI rendering tests pass.

**Step 2: Run the complete test suite**

Run:

```bash
cargo test --all-targets
```

Expected: all tests pass.

**Step 3: Inspect the final diff**

Run:

```bash
git diff --check
```

Expected: no whitespace errors; the diff contains only prompt-aware UI style selection, post-draw terminal style application, and focused regression tests.

**Step 4: Perform a manual terminal smoke test when an interactive terminal is available**

Launch LazyDB and verify:

- Normal mode displays a block cursor.
- Pressing `i` changes it to a vertical bar.
- Pressing Escape changes it back to a block.
- Pressing `:` from Normal mode displays a vertical bar in the prompt.
- Pressing Escape to close the prompt restores the block.
- Pressing `R` displays an underline.
- Exiting LazyDB restores the user's terminal cursor preference.

No commit should be created unless explicitly requested by the user.
