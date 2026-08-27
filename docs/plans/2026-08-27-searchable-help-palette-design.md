# Searchable Help Palette Design

## Objective

Turn the contextual help overlay into a keyboard-first searchable command palette:

- Show a search input at the top and keep the terminal cursor there while help is open.
- Select the first matching shortcut by default.
- Move selection with Up and Down and execute it with Enter.
- Close help before executing the selected shortcut.
- Represent every executable shortcut as one row. For example, `Ctrl-w h/j/k/l` becomes four independently selectable rows.

## Current Implementation

The current help overlay is static:

- `Overlay::Help(Focus)` stores only the focus captured when help opens.
- `render_help` in `src/ui/mod.rs` builds display-only `Line` values, including rows that combine several shortcuts and actions.
- `Keymap::map` treats help like a generic overlay, so only `Esc` and `q` dismiss it; text input, selection, and execution do not exist.
- Shortcut labels are maintained separately from the actual mappings in `src/input/keymap.rs` and the editor, so display and behavior can drift.
- The fixed 22-row popup is already too short for all Editor help entries.

## Chosen Approach

Use a structured shortcut catalog plus semantic execution.

Each help entry has a stable ID, one key label, one description, and a context. Rendering, filtering, selection, and Enter execution all use this catalog. Executing an item resolves its ID to an existing `Action` or an explicit editor key sequence; it does not simulate terminal input through `Keymap` and does not depend on the 750 ms pending-sequence timeout.

This approach is preferred over maintaining a second action table beside the current text because a parallel table can become misaligned with rendered rows. It is preferred over replaying raw terminal events because replay would couple execution to pending key sequences, editor mode, timing, and recursive event handling.

## State Model

Replace `Overlay::Help(Focus)` with `Overlay::Help(HelpState)`. `HelpState` contains:

- `context: Focus`: captured when help opens and used to build the contextual catalog.
- `query: String`: the current single-line search value.
- `selected: usize`: index in the filtered result set, initially zero.

Define `HelpShortcutId` and `HelpShortcut` in a dedicated help module. The ID is stored and passed by actions; it is stable independently of filtering and prevents an index from selecting the wrong item after the query changes.

`HelpShortcut` remains presentation-oriented and does not contain `Action`. The app reducer resolves `HelpShortcutId` to existing actions or editor keys. This avoids introducing an `Action` dependency into the workspace model.

## Catalog Rules

- One executable shortcut per row and per ID.
- Split all combined rows, not only `Ctrl-w h/j/k/l`.
- Keep global executable shortcuts before context-specific shortcuts.
- Remove non-executable information such as `Editor title` from the selectable list.
- Move help controls such as open/close instructions to a fixed footer. Do not include `F1`, `?`, or `Esc` as executable catalog entries because executing them would either reopen help or have ambiguous overlay semantics.
- Preserve prefix commands such as Editor `d` and `c` as prefix commands. Enter closes help and puts the editor into the same pending operator state as pressing that key normally; it does not invent a complete edit.
- Include only shortcuts valid for the captured context and active tab capability, such as relation-data commands.

The catalog is the in-app help source of truth. `docs/keybindings.md` remains the full user-facing contract and must use the same one-shortcut-per-row representation where actions differ.

## Search And Navigation

Help input is text-first:

- Printable characters, including `q`, `j`, and `k`, append to the query.
- Backspace removes the last character.
- `Ctrl-u` clears the query.
- Paste appends sanitized single-line text to the query.
- Up and Down move through filtered results and wrap at both ends.
- Enter executes the selected result.
- Esc closes help.

The query is split on whitespace and compared case-insensitively against a normalized string containing the key label and description. Every token must match. Thus `ctrl editor` matches both `Ctrl-w k  focus Editor` and `Ctrl-w l  focus Editor`. Results retain catalog order; there is no fuzzy score or unstable reordering.

Changing the query resets `selected` to zero. An empty result set shows `No matching shortcuts`; Enter has no effect.

## Execution Flow

Help-specific key handling runs before generic overlay handling in `Keymap::map` and clears any pending global key sequence.

Key events map to explicit reducer actions for inserting/deleting/clearing the query, moving selection, executing the selected item, and dismissing help. `ExecuteHelpShortcut` carries the stable selected ID, not the filtered index.

The reducer handles execution in this order:

1. Validate that help is still open and the ID is still the selected filtered entry.
2. Remove the help overlay.
3. Resolve the ID to an existing semantic action or editor key sequence.
4. Apply it through the existing reducer path and return all resulting runtime commands.

Closing first makes focus changes and newly opened overlays immediately visible. Existing reducers remain responsible for query execution, tab changes, profile operations, relation actions, and editor effects.

## Rendering

Keep the current visual language and `KEYMAP // CONTEXT` title. Divide the popup into:

1. A one-line `Search` input at the top.
2. A one-line spacer.
3. A single-line scrolling result list.
4. A fixed footer: `Up/Down select   Enter run   Esc close   Ctrl-u clear`.

The selected row uses `theme.selection`; the key and description keep their existing action/text colors. Rows do not wrap, preserving one item per line and stable selection geometry.

The popup keeps an approximately 74-column target width but derives its height from the available terminal area. The result viewport consumes the remaining inner height. Rendering derives the first visible row from `selected` and the current list height, so the selected row remains visible without storing terminal-dependent viewport state in the app.

While help is visible, rendering sets the terminal cursor at the end of the search value and sets `UiState.cursor_style` to `CursorStyle::Bar`. This overrides the underlying editor cursor and makes the search field the unambiguous visual focus.

Mouse selection is outside this change. Existing footer clicking can continue to open help.

## Edge Cases

- Help captures its context when opened; executing a focus shortcut can then change the underlying focus after help closes.
- Query edits use character counts and terminal display width when positioning the cursor, so Unicode paste cannot place the cursor inside a multi-byte character.
- Pasted tabs/newlines are normalized to spaces to keep the input single-line.
- If the terminal is too short to show a list row, render the search and footer where possible without panicking; execution still operates on the selected filtered entry.
- Key release events remain ignored.
- `q`, `j`, and `k` no longer close or navigate help because search input has priority.

## Verification

- Keymap tests cover help input priority, editing, Up/Down wrapping, Enter, Esc, and `q/j/k` as text.
- Paste tests cover single-line sanitization and appending to the help query.
- Reducer tests cover initial state, query reset behavior, empty results, stable-ID validation, close-before-execute, editor prefix execution, and command propagation.
- UI render tests cover the search field, bar cursor, default selection, four independent `Ctrl-w` rows, filtered output, empty output, and scrolling in a short terminal.
- Run formatting, focused tests, the complete test suite, and Clippy.
