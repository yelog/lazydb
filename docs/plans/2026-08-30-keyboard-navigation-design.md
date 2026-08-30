# Keyboard Navigation Design

## Scope

Align LazyDB's keyboard navigation for workspace panes, SQL consoles, and
relation result views with established Vim and terminal application habits.
The change is limited to key dispatch, help text, documentation, and tests.
It does not add a configurable keymap system or change database behavior.

## Design

### Workspace panes

`Ctrl-w h/j/k/l` represents geometric movement: left, down, up, and right.
The existing layout maps Explorer to the left, Editor to the upper right, and
Results to the lower right. Directional movement stops at an edge rather than
wrapping to a non-adjacent pane. `Tab` and `Shift-Tab` remain the global
cyclic-focus fallback when a user wants to move through all available panes.

The same directional meaning is used by the global keymap and the modal SQL
editor keymap. Help text describes the keys as directions rather than as
absolute pane names where that distinction matters.

### SQL console tabs

`gt` and `gT` are the primary next and previous console-tab bindings, matching
Vim's native tab-page conventions. `Ctrl-PageDown` and `Ctrl-PageUp` are
additional direct bindings for terminal and IDE users. Existing `[t` and `]t`
bindings remain aliases for this release to avoid breaking existing users.

Numeric tab selection is intentionally out of scope for this change.

### Result views

When Results is focused, numeric keys select a view directly:

- `1`: Data
- `2`: Output for SQL consoles, DDL for relation tabs
- `3`: Plan when that view exists

`o` remains the cycle/toggle binding. Relation-specific `p` and `D` aliases
remain available for Data and DDL. Direct numeric selection is scoped so it
does not interfere with editor input, explorer actions, grid navigation, or
data-query text fields.

## Error handling and compatibility

Unsupported numeric view selections are no-ops. Existing aliases continue to
dispatch their current actions. Input-mode-specific precedence remains intact:
editor Insert mode, data-query inputs, completions, and overlays consume their
own keys before workspace navigation.

## Verification

Add or update tests for directional pane dispatch, standard console-tab
bindings and their aliases, modifier-based page navigation, and direct result
view selection. Run formatting, targeted tests, the full test suite, and
Clippy if available in the repository's standard commands.
