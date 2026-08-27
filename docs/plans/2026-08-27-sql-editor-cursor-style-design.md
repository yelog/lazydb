# SQL Editor Cursor Style Design

## Problem

The SQL editor already derives a cursor style from its Vim mode, but the runtime no longer sends that style to the terminal after rendering. As a result, changing between Normal and Insert mode updates `UiState::cursor_style` without changing the visible terminal cursor.

This is a regression introduced when the runtime rendering entry point changed to `render_with_state_using_icons`. The previous post-render call to `TerminalSession::set_cursor_style` was dropped during that change.

## Expected Behavior

- Normal and visual modes use a steady block cursor.
- Insert mode uses a steady bar cursor.
- Replace mode uses a steady underline cursor.
- The `/`, `?`, and `:` editor prompts always use a steady bar cursor because they are text inputs independent of the current Vim mode.
- Exiting LazyDB restores the user's default terminal cursor shape.

## Design

Keep cursor-style selection in the UI layer and terminal control-sequence emission in the terminal/runtime layer.

`render_editor` records the desired style in `UiState::cursor_style`. A visible editor prompt takes precedence and selects `CursorStyle::Bar`; otherwise the style is derived from `EditorRenderSnapshot::mode`.

After the initial draw and after every subsequent redraw, the runtime reads `UiState::cursor_style` and passes it to `TerminalSession::set_cursor_style`. Reapplying the style after each actual redraw is intentional: focus changes or terminal behavior may reset the cursor shape, while the cost of emitting the short control sequence is negligible.

No editor input logic or mode transition code will perform terminal I/O. This preserves the existing boundary between editor state, UI projection, and terminal side effects.

## Alternatives Considered

### Apply the style inside `TerminalSession::draw`

This would make omission by callers less likely, but the draw API does not own `UiState`. Returning UI state from the render closure or passing it into the terminal abstraction would increase coupling and broaden the change unnecessarily.

### Apply the style during key handling

This would react directly to mode changes, but it would couple editor input handling to terminal I/O and miss non-keyboard transitions such as focus, overlay, prompt, and tab changes.

## Error Handling

Cursor-style emission returns `io::Result` and participates in the existing runtime error propagation. Terminal cleanup continues to issue `SetCursorStyle::DefaultUserShape` on drop.

## Verification

UI rendering tests should cover:

- Normal mode selects `Block`.
- Insert mode selects `Bar`.
- Escape from Insert mode returns to `Block`.
- An editor prompt selects `Bar`, including when opened from Normal mode.
- Replace mode selects `Underline`.

The focused UI test suite, formatting check, and project test suite should be run after implementation.
