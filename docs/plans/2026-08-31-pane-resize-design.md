# Pane Resize Design

## Scope

Add Vim-style workspace pane resizing to the existing `Ctrl-w` command family.
The feature covers the fixed Explorer, Editor, and Results layout; it does not
introduce arbitrary pane creation, pane removal, or a generic split tree.

The supported commands are:

- `Ctrl-w +`: increase the focused pane's height.
- `Ctrl-w -`: decrease the focused pane's height.
- `Ctrl-w >`: increase the focused pane's width.
- `Ctrl-w <`: decrease the focused pane's width.
- `Ctrl-w =`: restore both dimensions to their dynamic defaults.
- `N Ctrl-w +|-|>|<`: apply an explicit multi-digit step count.

The existing `1`, `2`, and `3` result-view shortcuts are removed so global
numeric prefixes can be recognized consistently. Result views remain reachable
through `o`, and relation tabs retain `p` and `D`.

## Current Architecture

The application does not currently have a general pane manager. The layout is
fixed in `src/ui/layout.rs`:

- Explorer occupies the left column.
- Editor occupies the upper-right area.
- Results occupies the lower-right area.
- Relation tabs replace the upper-right/lower-right pair with one right pane.
- Terminals narrower than 100 columns render only the focused pane.

Consequently, only two split boundaries need user-controlled state: Explorer
width and Editor height. A split tree would add abstractions and invalid states
without supporting a current product requirement.

`Ctrl-w` handling currently has two paths. Explorer and Results use the global
`Keymap`; Editor Normal mode uses `EditorWorkspace` so Vim editing commands keep
their precedence. Both paths must emit the same application-level resize
actions.

## State Model

Add session-level pane size preferences to `App`:

```rust
struct PaneSizePreferences {
    explorer_width: Option<u16>,
    editor_height: Option<u16>,
}
```

`None` means "calculate the dynamic default for the current terminal size."
This distinction is required so `Ctrl-w =` restores the existing responsive
defaults rather than copying dimensions from one terminal size.

The application also needs the latest effective split metrics produced by the
renderer. The metrics contain the actual Explorer width and Editor height, plus
whether each split exists. They follow the existing viewport synchronization
pattern: render computes geometry, and runtime dispatches a metrics action only
when the values change.

Each resize starts from the latest effective dimension and writes the clamped
result into the preference. This prevents invisible offset accumulation when a
user repeatedly resizes against a boundary.

Pane preferences are not persisted in a connection workspace. They are UI
session state shared across tabs and connections. Cross-process persistence is
outside this change.

## Layout Calculation

Extend `AppLayout::calculate` to accept pane size preferences.

When a preference is absent, preserve the current formulas:

- Explorer width: `(area.width / 3).clamp(34, 56)`.
- Editor height: 46 percent of the right-side content region.

When a preference is present, clamp it against the current render area:

- Explorer remains at least 34 columns wide.
- The right side remains at least 60 columns wide.
- Editor and Results each retain their minimum renderable content height.
- The fixed workspace-tab and result-tab rows are excluded before vertical
  split bounds are calculated.

The default Explorer maximum of 56 applies only to automatic sizing. A user may
grow Explorer beyond 56 while enough room remains for the right side.

Focus mode on terminals narrower than 100 columns has no active split, so resize
commands are no-ops. Preferences remain stored and become effective if the
terminal returns to Standard or Wide mode. Too-small mode behaves the same way.

Relation tabs have a horizontal split but no Editor/Results vertical split.
Width commands remain active and height commands are no-ops.

## Resize Semantics

Application actions represent a resize axis and signed split delta, plus a
separate reset action. Input adapters translate focused-pane semantics into the
split delta before dispatch:

| Focus | `+` | `-` | `>` | `<` |
| --- | --- | --- | --- | --- |
| Explorer | no-op | no-op | grow Explorer | shrink Explorer |
| Editor | grow Editor | shrink Editor | grow right side | shrink right side |
| Results | grow Results | shrink Results | grow right side | shrink right side |

Growing the right side shrinks Explorer; shrinking the right side grows
Explorer. Growing Results shrinks Editor; shrinking Results grows Editor.

All changes saturate at the current layout boundary. Counts are parsed with
checked arithmetic and capped to a value that can be safely converted to the
layout delta. A zero count and a count that cannot affect the split are no-ops.
No resize wraps focus, changes tabs, or creates deferred adjustment debt.

`Ctrl-w =` clears both optional preferences. It does not equalize panes. The
next layout pass immediately uses the responsive default formulas.

## Input State Machines

### Explorer and Results

The global keymap recognizes a multi-digit prefix before `Ctrl-w`. After
`Ctrl-w`, it accepts focus movement, the four resize operators, and reset.
Pending state remains bound to the active focus, editor mode, tab, and sequence
timeout as it is today.

Digits no longer switch result views. If a numeric prefix is not followed by a
valid window command, the pending count is cleared and no result-view action is
dispatched.

### Editor

Editor Normal mode must preserve native Vim counts such as `5j`, `10G`, and
`0`. It therefore temporarily buffers leading digits:

- If the next command begins with `Ctrl-w`, consume the digits as the pane
  resize count.
- Otherwise replay the complete buffered sequence to the existing modalkit
  input path before processing the current key.
- Preserve Vim's special handling of a leading `0` when it is not part of a
  non-zero count.

Editor Insert and Replace modes keep their current behavior: `Ctrl-w` deletes
the previous word and never starts a pane command.

Both input paths emit the same reducer actions. `App` remains the only owner of
pane size preferences.

## Help And Documentation

Add all resize commands and count syntax to contextual help and
`docs/keybindings.md`. Remove help entries and documentation for direct
`1`/`2`/`3` result-view selection. Keep `o`, `p`, and `D` documented as the
remaining result-view navigation commands.

## Verification

Layout tests cover dynamic defaults, explicit preferences, width and height
bounds, user widths above the automatic maximum, narrow focus mode, relation
tabs, terminal resizing, and reset behavior.

Global keymap tests cover each operator, reset, multi-digit prefixes, zero and
overflow counts, invalid sequence cancellation, timeout/context invalidation,
and removal of direct numeric result-view selection.

Editor tests cover counted pane effects and regression cases for `5j`, `10G`,
leading `0`, Insert-mode `Ctrl-w`, and existing `Ctrl-w h/j/k/l` focus commands.

Reducer tests cover signed split direction for every focus, relation and narrow
mode no-ops, boundary saturation without hidden offsets, and clearing both
preferences on reset.

Run `cargo fmt`, targeted layout/keymap/editor/reducer tests, the full test
suite, and `cargo clippy`.
