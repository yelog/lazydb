# Contextual Key Hints Design

## Goal

Make the footer teach the most useful shortcuts for the currently focused pane,
temporarily replace those shortcuts with valid continuations while a key
sequence is pending, and keep the complete keyboard contract in a dedicated
document rather than inline in the README.

## Current State

Shortcut information currently has four manually maintained representations:

- `src/input/keymap.rs` implements the actual input behavior.
- `src/help.rs` contains the contextual help rows.
- `src/ui/mod.rs` hard-codes one footer hint string per `Focus`.
- `docs/keybindings.md` documents the user-facing keyboard contract.

These representations already differ. The footer cannot distinguish SQL data,
SQL output, Relation Data, Relation DDL, editor modes, input states, or overlays.
It also cannot react to a pending sequence because `Keymap::pending` is private
runtime state while the UI only receives `App`.

Pending sequences expire after 750 ms. A prefix currently returns no `Action`,
so the runtime neither redraws when the sequence begins nor redraws when it
expires.

## Chosen Approach

Keep `src/input/keymap.rs` as the authority for executable behavior and create a
shared display catalog for footer hints and contextual help. Do not replace the
existing input mapper with a declarative binding engine: its context-dependent
guards, text input states, Vim behavior, and `modalkit` integration would make
that substantially larger and riskier than the requested feature.

The display catalog is metadata, not an execution mechanism. Tests will keep
its prefix definitions aligned with the input mapper.

## Shortcut Catalog

Generalize the shortcut data currently in `src/help.rs` so each entry can
describe:

- A stable shortcut identifier.
- The context in which it is available.
- Its complete display sequence, such as `<space> n` or `<ctrl+w> h`.
- Its suffix after a recognized prefix, such as `n` or `h`.
- A concise description.
- An optional footer priority.
- Any simple availability condition needed to hide an inapplicable hint.

The catalog must distinguish at least these contexts:

- Global workspace.
- Explorer.
- SQL Editor Normal, Insert/Replace, and Visual modes.
- SQL Results Data and SQL Output.
- Relation Data browse, cell edit, and Visual Line modes.
- Relation DDL.
- Record View.
- Data Query input.
- Profile Manager form, scope, and delete confirmation.
- SQL Editor List.
- Help search.
- Confirmation overlays.

The active display context is derived from `App` in this order:

1. Active overlay or modal.
2. Active input or edit sub-state.
3. Active tab and view.
4. Focused pane and editor mode.

This prevents a Results footer from describing grid cells while SQL Output or
Relation DDL is active.

## Pending Sequence View

Keep the internal `Pending` enum private to the keymap. Expose a semantic,
read-only sequence snapshot instead:

```rust
pub struct KeySequenceState {
    pub prefix: ShortcutPrefix,
    pub display: String,
    pub remaining: Duration,
}
```

`Keymap::sequence_state(app, now)` returns a snapshot only when the sequence:

- Has not exceeded the 750 ms timeout.
- Still belongs to the current focus.
- Still belongs to the current editor mode.
- Still belongs to the active tab.

The public prefix vocabulary covers all existing pending sequences, including:

- `<space>` leader commands.
- `<ctrl+w>` focus and resize commands.
- `g` navigation.
- `z` alignment.
- `[` and `]` tab aliases.
- Relation `d` and `y` operations.
- Record View `g` navigation.

Counted pane commands retain their existing input behavior. Before `<ctrl+w>` is
entered, a numeric count does not display all window candidates. Once the
window prefix is complete, the footer includes the count in its prefix label.

## Runtime Data Flow

The runtime compares the sequence snapshot before and after every keyboard
event. It redraws whenever the snapshot changes, even when `Keymap::map`
returns no `Action`.

The 33 ms ticker also detects when a visible sequence expires and triggers one
redraw to restore normal hints. Mouse, paste, resize, and terminal focus events
continue to clear pending input and redraw when that changes the visible
sequence.

The sequence remains runtime input state and is not copied into `App`. The
runtime passes an optional snapshot into the top-level UI render call. Existing
test and convenience render functions may default to no pending sequence.

## Footer Behavior

The footer remains two rows tall:

- Row one contains the mode/context badge and keyboard hints.
- Row two retains readiness, errors, clipboard notices, and relation provenance.

When no sequence is pending, row one shows high-frequency shortcuts selected
from the active catalog context. When a sequence is pending, the complete hint
area is replaced by all valid continuations for that prefix and context.

Hints are indivisible layout units. The renderer measures terminal cell width,
adds complete hints in priority order, and never allows Ratatui to cut a hint in
the middle. If all hints do not fit, it appends `... (+N)`. If the terminal is
too narrow for one candidate, it preserves the prefix label and omitted count.
The footer never grows vertically, so pane geometry does not jump while typing.

## Default Footer Hints

The exact set is width-dependent, but priority starts with these actions.

### Explorer

- `j/k` move.
- `h/l` collapse or expand.
- `Enter` open.
- `/` find visible nodes.
- `f` search the catalog.
- `r` refresh.
- `F1` help.

### SQL Editor Normal

- `i` enter Insert mode.
- `F5` run the current statement.
- `Shift-F5` run the complete buffer.
- `<space> f` format SQL.
- `<space> y` copy the current statement.
- `F1` help.

### SQL Editor Insert or Replace

- `Esc` return to Normal mode.
- `Ctrl-Space` trigger completion.
- `Ctrl-w` delete the previous word.
- `F5` run SQL.
- `F1` help.

`Ctrl-w` in Insert/Replace mode must not be presented as a pane prefix.

### SQL Editor Visual

- `y` copy selection.
- `Esc` return to Normal mode.
- `F5` run selection.
- `<space> f` format selection.
- `F1` help.

### SQL Results Data

- `h/j/k/l` move between cells.
- `v` open Record View.
- `y` copy cell.
- `Y` copy row.
- `o` open Output.
- `/` focus WHERE when data-query capability is available.
- `F1` help.

### SQL Output

- `j/k` move.
- `gg/G` move to ends.
- `/` search.
- `v/V` select.
- `y` copy.
- `o` return to Data.
- `F1` help.

### Relation Data

Editable browse mode prioritizes movement, `i` edit, `a` insert, `V` select
rows, `p` paste, and `Ctrl-s` commit. Read-only or unavailable editing contexts
instead prioritize movement, Record View, copying, WHERE, ORDER BY, and refresh.

Cell edit mode shows `Enter` apply, `Esc` cancel, and text-edit controls. Visual
Line mode shows `j/k` extend, `y` yank, `d` delete, and `V` cancel.

### Relation DDL

- `j/k` move.
- `gg/G` move to ends.
- `/` search.
- `v/V` select.
- `y` copy.
- `p` return to Data.
- `r` refresh.

### Record View

- `j/k` move through fields.
- `h/l` move through records.
- `gg/G` move to first or last field.
- `Esc` close.

Overlays and focused text inputs replace pane hints with their own applicable
controls.

## Prefix Hints

Prefix candidates are filtered by active context and actual availability.
Examples include:

```text
<space> n new SQL   s first SQL   r run   R run all   f format   d target   ... (+N)
<ctrl+w> h Explorer   j Results   +/- height   >/< width   = reset
g g first row   t next tab   T previous tab
z z center row   t top   b bottom
[ t previous tab
] t next tab
d d delete row
y y yank row
```

Window focus candidates omit impossible destinations. For example, Relation
tabs have no Editor pane, so Results does not advertise `<ctrl+w> k` there.

## Documentation

Use the existing `docs/keybindings.md` as the single dedicated keyboard
reference. Rewrite and verify it against the input mapper, including all pane,
mode, input, overlay, and confirmation contexts.

Organize it as:

1. Conventions.
2. Global.
3. Pane navigation and resize.
4. Prefixes.
5. Explorer.
6. SQL Editor.
7. SQL Results Data.
8. SQL Output.
9. Relation Data.
10. Relation DDL.
11. Record View.
12. Data Query inputs.
13. Profile Manager.
14. SQL Editor List.
15. Help search.
16. Confirmation dialogs.
17. Mouse.

Remove the Essential Keys table and all concrete key listings from README.
Replace them with a short statement that the footer shows contextual controls
and a link to `docs/keybindings.md`. Keep the Keyboard Reference entry in the
README documentation table.

## Verification

Add tests for:

- Unique shortcut catalog identifiers.
- Prefix entries whose complete sequence and suffix agree.
- Candidate sets for `<space>`, `<ctrl+w>`, `g`, `z`, `[`, `]`, Relation `d`,
  Relation `y`, and Record View `g`.
- Context-sensitive window direction filtering.
- Insert-mode `Ctrl-w` remaining an editor command rather than a pane prefix.
- Sequence invalidation after timeout, focus change, mode change, and tab change.
- Immediate redraw when a prefix starts and one redraw when it expires.
- Default hints for every principal pane, view, edit mode, and overlay.
- Complete-unit truncation and accurate `... (+N)` output at 56, 80, and 120
  columns.
- README containing only the keyboard reference link, not a key table.
- The dedicated reference containing every required context section.

Run `cargo fmt --check`, targeted keymap and UI tests, `cargo clippy
--all-targets -- -D warnings`, and the complete `cargo test` suite.
