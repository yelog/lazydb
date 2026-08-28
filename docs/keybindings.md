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
| `Ctrl-w h` | Focus Explorer |
| `Ctrl-w j` | Focus Results |
| `Ctrl-w k` | Focus Editor |
| `Ctrl-w l` | Focus Editor |
| `Tab`, `Shift-Tab` | Next/previous panel outside Insert mode |
| `[` then `t`, `]` then `t` | Previous/next LazyDB tab |
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
| `gg`, `G` | Select the first/last row |
| `H`, `M`, `L` | Select the top/middle/bottom visible row |
| `Ctrl-d`, `Ctrl-u` | Move down/up half a page |
| `Ctrl-f`, `Ctrl-b` | Move down/up one page |
| `zz`, `zt`, `zb` | Align the selected row to the middle/top/bottom |
| `o` | Switch Data/Output |

Page movement moves the selected row with the viewport. The fixed `#` column
shows one-based absolute row numbers and remains visible during horizontal
scrolling.

Relation Data previews additionally support `/` to focus the `WHERE` input and
`s` to focus `ORDER BY`. Press `Enter` to apply both clauses, or `Esc` to leave
the inputs without applying drafts. `[` and `]` resize the selected column and
`=` restores its automatic width. Preview queries retain the hard 500-row limit.

Selecting a relation or one of its supported descendants opens a relation tab.
Relation tabs have `DATA` and `STRUCTURE` pages. Data uses an adapter-generated,
read-only preview with a hard `LIMIT 500`; the adapter owns the SQL and the
limit. Zero-row results still retain and render column metadata. `r` retries a
failed relation request and `Ctrl-C` cancels an in-flight relation request.
Relation snapshots identify whether they are `LIVE`, `OFFLINE SNAPSHOT`,
`PROFILE DELETED SNAPSHOT`, or `OUT OF SCOPE SNAPSHOT`.

## Mouse

- Left click switches panels, activates tabs, selects catalog rows, selects result
  cells, and toggles Data/Output.
- Wheel scroll moves the panel under the pointer.
- Closing the Neovim floating window hides it without stopping LazyDB.
# Workspace lifecycle

- `Space q`: close the current tab; SQL editors are hidden and retained.
- `Space x`: request confirmed permanent deletion of the current SQL editor.
- `Space e`: search and activate any persisted SQL editor, including hidden editors.
