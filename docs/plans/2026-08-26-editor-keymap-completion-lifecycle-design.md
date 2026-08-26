# Editor Keymap and Completion Lifecycle Design

**Status:** Approved

**Date:** 2026-08-26

## Summary

LazyDB will make key ownership explicitly mode-aware and will separate completion
replacement scanning from automatic-completion eligibility. Normal-mode global
keys remain global even if a stale completion popup exists. Insert-mode Escape
always reaches the editor, which already clears the popup before changing the
modal state. Automatic completion stops after delimiters and closed quoted
identifiers while remaining available for unfinished identifiers and qualifier
dots.

## Confirmed Behavior

- Normal `?` opens contextual Help.
- Normal Tab focuses the next panel.
- Normal Shift-Tab focuses the previous panel.
- These global keys win even if a completion popup is visible.
- Insert Escape closes completion and exits Insert in one keypress.
- Completion owns Ctrl-N, Ctrl-P, and Enter, but not Escape.
- `tools.sys|` may offer `sys_config`.
- Accepting it produces `tools."sys_config"|` with no remaining popup.
- `tools.|` may offer qualified children.
- Closed quotes, whitespace, commas, brackets, parentheses, and semicolons stop
  automatic completion.
- Explicit Ctrl-Space completion remains available independently of automatic
  trigger eligibility.

## Keymap Ownership

After modal overlays, `Keymap::map` applies this precedence:

1. Normal-mode global keys.
2. Insert-mode Escape.
3. Completion navigation and acceptance.
4. Other global keys.
5. Focus- and mode-specific ordinary input.

Normal-mode global keys are:

```text
?          ShowHelp
Tab        FocusNext
Shift-Tab  FocusPrevious
```

Insert Escape maps to `Action::EditorKey(Esc)`. `App::update` already clears the
active popup before calling `EditorWorkspace::key`, so this one action both
dismisses completion and transitions modalkit to Normal. No compound action or
second mode transition is added.

When a popup is visible, it owns only:

```text
Ctrl-N     CompletionNext
Ctrl-P     CompletionPrevious
Enter      CompletionAccept
```

All other keys continue through normal mode-aware routing.

## Completion Eligibility

Add one predicate used by automatic completion scheduling and immediate refresh:

```rust
fn should_offer_completion(text: &str, cursor: usize) -> bool
```

It returns true when:

- the cursor follows an unfinished identifier continuation character; or
- the cursor follows a qualifier dot; or
- the caller explicitly requested completion.

It returns false after a closed quote, string delimiter, whitespace, comma,
closing bracket or parenthesis, semicolon, or newline.

Replacement scanning and eligibility are distinct concepts:

```rust
is_identifier_scan_byte
is_completion_continuation
```

Replacement scanning may include `"` and backticks so incomplete quoted
identifiers have the correct replacement range. Completion continuation does not
treat a closing quote as an unfinished identifier character.

## Acceptance Flow

Completion replacement remains a normal undoable editor edit and increments the
document revision. Its `Changed` effect is handled normally. The App consults the
eligibility predicate before scheduling another popup:

```text
Changed
  -> clear stale popup
  -> should_offer_completion(text, cursor)
  -> schedule only when true
```

No `just_accepted_completion` flag is introduced. This also handles manually
typed delimiters and allows a later dot to start qualified completion again.

## Tests

Keymap tests cover:

- Normal `?`, Tab, and BackTab with and without a popup.
- Insert Escape with a popup maps to `EditorKey(Esc)`.
- Popup Ctrl-N, Ctrl-P, and Enter remain completion actions.

App tests cover:

- Insert Escape clears the popup, enters Normal, and leaves text unchanged.
- Accepting a quoted catalog candidate leaves the cursor at the right edge and
  leaves no popup.

Completion tests cover:

- unfinished qualified prefix;
- qualifier dot;
- closed PostgreSQL quote;
- closed MySQL backtick;
- whitespace, comma, parenthesis, and semicolon boundaries;
- explicit completion bypassing the automatic eligibility predicate.

## Acceptance Criteria

1. Normal `?`, Tab, and Shift-Tab always retain their global behavior.
2. Insert Escape exits Insert even while completion is visible.
3. Completion acceptance does not immediately reopen the same single candidate
   after a closed quoted identifier.
4. Qualified completion after a dot still works.
5. Existing Vim, completion navigation, cursor placement, undo, and popup
   positioning tests remain green.
