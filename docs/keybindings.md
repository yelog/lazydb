# Keybindings

This document lists the operational keyboard contract. The in-app footer shows
the shortest relevant subset; `F1` works everywhere and `?` is backward search in
Editor Normal mode but contextual help outside the editor.

## Global

When contextual help is open, its search field has focus. Printable text is
searched case-insensitively, Up/Down selects a result, Enter executes it after
closing the overlay, Esc closes the overlay, Backspace deletes the last query
character, and Ctrl-U clears the query. The help list has one row per
executable shortcut, so shortcuts with different actions are independently
selectable.

| Key | Action |
| --- | --- |
| `F1` | Contextual help |
| `?` | Contextual help outside Insert mode |
| `Ctrl-w h/j/k/l` | Move focus left/down/up/right |
| `Ctrl-w +` / `Ctrl-w -` | Increase/decrease focused pane height |
| `Ctrl-w >` / `Ctrl-w <` | Increase/decrease focused pane width |
| `Ctrl-w =` | Restore responsive default pane sizes, not equalize |
| `N Ctrl-w +|-|>|<` | Apply a counted pane resize; unsupported dimensions are no-ops |
| `Tab`, `Shift-Tab` | Next/previous panel outside Insert mode |
| `gT`, `gt` | Previous/next LazyDB tab |
| `Ctrl-PageUp`, `Ctrl-PageDown` | Previous/next LazyDB tab |
| `[` then `t`, `]` then `t` | Compatibility aliases for previous/next LazyDB tab |
| `Space n` | New SQL console |
| `Space s` | Go to the first available SQL console and focus its editor |
| `Ctrl-c` | Cancel active query; otherwise leave Insert mode |
| `Q` | Quit LazyDB in Normal mode |
| `Space c` | Focus the connection Explorer |

## Profile Manager

| Key | Action |
| --- | --- |
| `j/k`, Up/Down | Previous/next non-text form field; Up/Down also leave text fields |
| `Tab`, `Shift-Tab` | Next/previous form field |
| `h/l`, Left/Right | Previous/next driver, URL format, SSL mode, or environment |
| `Enter` | Apply the URL field or activate the selected form action |
| `n` | Create a new profile |
| `t` | Test Connection without saving |
| `s` | Save the profile without connecting |
| `c` | Save & Connect |
| `d` | Delete after confirmation |
| `Space` | Toggle checkboxes and SQLite memory mode |
| `Esc` | Close, cancel, or leave the manager |

The manager edits one draft at a time; it does not open a profile-list popup.
The Explorer is the profile navigation surface. Its roots are keyed by profile
UUID; temporary roots show `SESSION` while saved roots have no provenance label.
Connection state is shown with a status marker, and process/error states may also
show text. With no roots, select `No profiles` to start a new draft. Root
refresh/connect/retry actions apply only to the selected UUID. Catalog status
rows include loading, stale/retry, and permission-denied/retry states.

`Test Connection` is non-persistent and leaves the active connection alone. On
success it discovers databases and schemas used by the hierarchical scope
picker. The picker supports `All` and `Selected`; MySQL displays a read-only
database-as-schema mirror rather than independently selectable schemas.

DRIVER displays PostgreSQL, MySQL, and SQLite horizontally, with the current
choice highlighted. In text fields, `h/j/k/l` remain literal input; Left/Right
move the text cursor and Up/Down change fields. PostgreSQL exposes an optional
default schema. URL FORMAT cycles only through formats compatible with the
selected driver, and URL applies on Enter, field exit, Test, or Save.

## Explorer

| Key | Action |
| --- | --- |
| `j/k`, arrows | Move selection |
| `gg`, `Home` | First visible object |
| `G`, `End` | Last visible object |
| `H/M/L` | Top/middle/bottom object in the current viewport |
| `Ctrl-f/Ctrl-b` | Move down/up one page |
| `Ctrl-d/Ctrl-u` | Move down/up half a page |
| `zz/zt/zb` | Align current selection to middle/top/bottom |
| `Home/End` | First/last visible object |
| `/` | Find within the currently visible Explorer tree |
| `f` | Search all objects in the active catalog scope |
| `n/N` | Next/previous confirmed `/` or `f` result |
| `h/l`, left/right | Collapse or expand |
| `o` | Toggle expansion only |
| `Enter` | Activate; open the owning table/view preview for relations and descendants |
| `n` | Create a connection profile |
| `e` | Edit the selected connection profile |
| `c`, `x` | Connect/disconnect the selected profile |
| `d` | Delete the selected connection profile |
| `r` | Reload catalog |
| `p` | Open a 500-row table/view preview |
| `D` | Open available object DDL in a new SQL tab |
| `y` | Copy the selected node's primary name |
| `s` | Open connection access menu |

Expanding a database loads schemas, expanding a schema loads object groups, and
expanding a group loads objects. Catalog pages are lazy and may show `Load
more...`; `r` refreshes the selected UUID-owned target. A refresh can retain the
previous page as stale data until the replacement arrives. Late pages whose
connection identity, catalog epoch, request id, target, or cursor no longer
matches are ignored.

`/` finds primary node labels in the visible expanded-tree snapshot only. It does
not load or search descendants hidden by collapsed nodes or missing from loaded
pages. Matching is case-insensitive and highlights matching text in the normal
tree. Type to edit, press `Enter` to confirm, then use `n/N` to cycle results;
`Esc` clears the find state. `Backspace` deletes and `Ctrl-U` clears while editing.

`f` searches every actual object in the active connection's configured catalog
scope, including objects and relation children not loaded in the lazy tree.
Matching is case-insensitive over names and qualified paths. Results preserve the
ancestor and presentation-group tree structure and highlight matching labels.
Type to edit, use `j/k`, arrows, `Home/End` to select, `Enter` to locate and retain
a hit in the normal tree, `n/N` to cycle matching objects after confirmation, and
`Esc` to close. `Backspace` deletes and `Ctrl-U` clears. Failed searches use `r` to
retry. Results are limited to 100; refine a truncated search.

## SQL Editor

Normal mode:

| Key | Action |
| --- | --- |
| `h/j/k/l`, arrows | Move cursor |
| `i` | Insert at cursor |
| `a` | Insert after cursor |
| `o` | Open line below |
| `x`, `Delete` | Delete character |
| `0`, `$`, `Home`, `End` | Start/end of line |
| `F5`, `Space r` | Execute the selected/current statement |
| `Shift-F5`, `Space R` | Preview and execute the complete buffer |
| `Space f` | Format the selected/current statement |
| `Space y` | Copy the current SQL statement |
| `Space Y` | Copy the complete SQL buffer |
| `Ctrl-Space` | Trigger completion |
| `Space d` | Select the active console's database/schema execution target |
| `Ctrl-N/P` | Move through an open completion popup |
| `?`, `n`, `N` | Backward search and repeat |
| `F1`, `Space ?` | Editor help |
| `Space tt` | Toggle AUTO/MANUAL transactions |
| `Space tc` | Commit the active MANUAL transaction |
| `Space tr` | Roll back the active MANUAL transaction |

Insert mode:

| Key | Action |
| --- | --- |
| `Esc`, idle `Ctrl-c` | Return to Normal mode |
| `Tab` | Insert a tab character |
| arrows, Home, End | Move cursor |
| Backspace, Delete, Enter | Edit text |
| `Ctrl-W/U/H` | Delete word/to line start/backspace |

Visual selection takes precedence over the cursor statement. Empty selections do
not fall back to the whole buffer. Full-buffer execution is explicit and always
requires confirmation. `:run`, `:runall`, `:format`, `:s`, `:tx auto`, `:tx manual`,
`:tx clear`, `:commit`, and `:rollback` provide command-line equivalents.

MANUAL transactions use one pinned physical connection per console. Cancelling a
MANUAL query rolls back the complete transaction; MySQL DDL may implicitly commit.
`Space d` lists only targets discovered for the active profile and permitted by
its catalog scope. Use `j/k` or Up/Down to wrap through targets, Enter to confirm,
and Esc to cancel. Target changes are blocked while any query or MANUAL
transaction is active; a failed target connection leaves the previous target and
connection unchanged.

## Results

| Key | Action |
| --- | --- |
| `h/j/k/l`, arrows | Move selected cell |
| `0` / `^` / `$` | Select the first/last column in the current row (`0` and `^` select first) |
| `gg`, `G` | Select the first/last row |
| `H`, `M`, `L` | Select the top/middle/bottom visible row |
| `Ctrl-d`, `Ctrl-u` | Move down/up half a page |
| `Ctrl-f`, `Ctrl-b` | Move down/up one page |
| `zz`, `zt`, `zb` | Align the selected row to the middle/top/bottom |
| `v` | Open read-only Record View for the selected row |
| `y` | Copy the complete selected cell value |
| `Y` | Copy the selected row as TSV |
| `Space Y` | Copy the selected row with column headers as TSV |
| `o` | Switch Data/Output |

When `OUTPUT` is active, it is a read-only Vim text view rather than a grid:
`h/j/k/l`, arrows, `H/M/L`, `gg/G`, `Ctrl-d/u`, `Ctrl-f/b`, `/`, `?`, `n/N`,
`v`, `V`, and `Ctrl-v` navigate or select text. `y` copies the Visual selection
and `yy` copies the current line. Output status markers are visual decorations;
only the log message text is copied. Editing, paste, undo, redo, substitution,
and other mutation commands are disabled. Output follows the newest entry until
the user navigates or starts a selection, after which new entries do not move
the cursor or selection.

Page movement moves the selected row with the viewport. The fixed `#` column
shows one-based absolute row numbers and remains visible during horizontal
scrolling.

Record View shows fields in result-column order with their database types and
bounded value previews. Inside it, `j/k` scroll fields, `h/l` browse records,
`gg/G` jump to the first/last field, and `Esc`, `q`, or `v` closes the view.
It is read-only and does not load complete LOB values or execute database I/O.

Relation Data previews additionally support `/` to focus the `WHERE` input and
`s` to focus `ORDER BY`. Press `Enter` to apply both clauses, or `Esc` to leave
the inputs without applying drafts. `[` and `]` resize the selected column and
`=` restores its automatic width. Preview queries retain the hard 500-row limit.

Selecting a relation or one of its supported descendants opens a relation tab.
Relation tabs have `DATA` and `DDL` pages. The relation-local shortcuts are:

| Key | Action |
| --- | --- |
| `D` | Switch to the adapter-owned relation DDL page |
| `p` | Switch to the adapter-owned relation Data preview |
| `o` | Toggle between Data and DDL |
| `1`, `2` | Select Data or DDL directly |
| `r` | Refresh the active relation; retry a failed request |
| `j/k/h/l`, arrows | Move Data selection or move the DDL read-only Vim cursor |
| `gg`, `G` | Jump the DDL cursor to the beginning/end |
| `H/M/L` | Move the DDL cursor to the top/middle/bottom of its viewport |
| `Ctrl-d/u`, `Ctrl-f/b` | Move the DDL cursor by half/page viewport |
| `v`, `V`, `Ctrl-v` | Select DDL text in Visual Char/Line/Block mode |
| `y`, `yy` | Copy the selected DDL text or current line |

Data is a read-only, adapter-owned preview with a hard `LIMIT 500`, including
the SQL construction and limit. Zero-row results still retain and render column
metadata. DDL is a complete adapter-owned result; the UI does not reconstruct it
from catalog rows. On DDL, the content is a read-only Vim buffer. Normal/Visual
navigation, search, and yank are available. `V` enters Visual Line mode;
selection is visible while all editing,
paste, undo, redo, substitution, and write-oriented commands are disabled. DDL
status and provenance decorations are not part of selectable or copied text.
On Data, the same movement keys select a cell and the shared grid follows the
selected row/column only as needed. `0` and `^` select the first column, while
`$` selects the last column of the current row.

The mouse wheel scrolls the panel under the pointer: Explorer and SQL editor
move by three rows, Data moves the grid by three rows, and DDL moves its
vertical viewport by three rows. Left/right DDL movement remains keyboard-only.
`Ctrl-C` cancels an in-flight relation request. Relation snapshots identify
whether they are `LIVE`, `OFFLINE SNAPSHOT`, `PROFILE DELETED SNAPSHOT`, or `OUT
OF SCOPE SNAPSHOT`; this is snapshot provenance and is separate from DDL's
`NativeCatalog` or `AdapterGenerated` provenance.

## Mouse

- Left click switches panels, activates tabs, selects catalog rows, selects result
  cells, and toggles Data/Output.
- Application copy uses the semantic target under the cursor. It copies complete
  values even when the Grid preview is truncated. Terminal-native text selection
  commonly uses Shift-drag while mouse capture is enabled; the exact modifier is
  terminal-specific. Use `--mouse off` to prefer terminal-native selection.
- Wheel scroll moves the panel under the pointer; relation DDL scrolls vertically
  by three rows and relation Data scrolls the grid by three rows.
- Closing the Neovim floating window hides it without stopping LazyDB.
# Workspace Lifecycle

- `Space q`: close the current tab; SQL editors are hidden and retained.
- `Space x`: request confirmed permanent deletion of the current SQL editor.
- `Space e`: search and activate any persisted SQL editor, including hidden editors.

Workspaces are per connection profile. Switching profiles changes the visible
workspace only after the connection succeeds; a failed switch leaves the current
connection and workspace unchanged. Disconnecting hides the profile's workspace
without deleting it. Relation tabs restore as lazy shells, and their result data
is not persisted across an application restart. Deleting a profile removes its
workspace.
