# Keyboard Reference

This document is the keyboard contract for the current TUI. The footer is a
width-aware summary, not a separate binding system. Help rows and footer rows
come from the same shortcut catalog, while executable behavior remains in the
input mapper and editor.

The built-in configuration selects this complete contract with
`keybindings.preset = "vim"` in [`config/default.toml`](../config/default.toml).
The current release supports the Vim preset as a whole rather than arbitrary
per-command rebinding. The global command bindings listed in
The grouped keybinding tables are the current exception and accept single key
events or space-separated key sequences. They are organized as `global`,
`leader`, `panes`, `explorer`, `results`, `editor`, and `overlays` according to
the active panel. `keybindings.sequence_timeout_ms`
configures the timeout for adjacent-key sequences used by this preset.

The navigation commands for Explorer and SQL Results are also configurable.
Their default `j/k/h/l` bindings are scoped to their respective context, so
the same key can safely mean different movements in different views.

The configurable Leader commands currently include `Space b` (Dashboard),
`Space c` (Explorer), `Space s` (console manager), `Space r/R` (run the current
statement or complete buffer), and `Space d` (execution target selector).

## Conventions

`Ctrl-x` means hold Control while pressing `x`; `Shift-F5` means hold Shift
while pressing F5. Adjacent-key sequences have a 750 ms timeout. A prefix that
expires is discarded. A numeric count is displayed while it is being entered;
after `Ctrl-w`, it applies to pane resize operations.

The application Leader is used by Explorer and Results, but not by the SQL
Editor. In the SQL Editor, Normal and Visual Space bindings are handled by the
editor's modalkit `EditorLeader` binding and are forwarded through `EditorKey`;
they are not application Leader commands. Consequently, `Space c` is valid in
Explorer/Results, but it is not a way to focus Explorer from the SQL Editor;
use `Ctrl-w h` there instead. The editor also accepts `\` as an EditorLeader
prefix. Editor Insert Space remains text/completion input. A footer item can be
display-only when the underlying mapper has no direct action for that
presentation.

## Global

| Keys | Behavior |
| --- | --- |
| `F1` | Open contextual Help |
| `?` | Open Help outside Editor search/input states |
| `Ctrl-c` | Quit globally; context-specific cancellation may also be offered by its modal |
| `Tab` / `Shift-Tab` | Move focus between workspace panes where applicable |
| `Ctrl-PageUp` / `Ctrl-PageDown` | Previous/next workspace tab |
| `Q` | Quit from Editor Normal mode |

## Schema Owner Picker

When the New Schema Owner field has loaded PostgreSQL roles, the contextual
owner picker opens on `Enter`, on typing, and whenever the Owner field takes
focus through `Tab`, `Shift-Tab`, or a mouse click. These keys are handled by
the catalog editor and are not global configurable bindings.

| Keys | Behavior |
| --- | --- |
| `↑` / `↓` | Move through filtered owner roles |
| Typing / `Backspace` / `Ctrl-w` / `Ctrl-u` | Edit the owner-role filter |
| `Enter` | Choose the highlighted selectable role |
| `Esc` | Close the owner picker without closing the editor |
| `Tab` | Close picker and move to Comment |
| `Shift-Tab` | Close picker and move to Name |

Choosing a role closes the list and keeps focus on the Owner field. Every key the
picker does not own falls through to the catalog editor form, so Owner still
answers `Tab` / `Shift-Tab` field navigation, `Enter` to reopen the role list,
and `Esc` to cancel the editor. Moving focus off the Owner row releases the list
as well, and the list header echoes the active filter plus any message about a
role this session cannot assume.

## Pane Navigation and Resize

`Ctrl-w` is an application pane prefix outside Editor Insert/Replace and outside
editor-owned binding states. Candidates depend on the current layout.

| Context | Valid directions |
| --- | --- |
| SQL Explorer | `l` to Editor |
| SQL Editor | `h` to Explorer, `j` to Results |
| SQL Results | `h` to Explorer, `k` to Editor |
| Relation Explorer | `l` to Results |
| Relation Data, DDL, or Busy | `h` to Explorer |

`Ctrl-w +`, `Ctrl-w -`, `Ctrl-w >`, and `Ctrl-w <` resize the corresponding
focused split. `Ctrl-w =` restores default pane sizes. Unsupported directions
are not advertised. Counts are accepted before `Ctrl-w`, for example
`10 Ctrl-w +`.

`Ctrl-w Ctrl-w` cycles clockwise through panes that exist in the active layout.
SQL uses `Explorer -> Editor -> Results -> Explorer`; Relation and Dashboard
use `Explorer -> Results -> Explorer`.

## Prefixes

The catalog records prefixes explicitly; this reference does not infer them
from display strings.

| Prefix | Continuations |
| --- | --- |
| Application `Space` (Explorer/Results) | `c`, `n`, `s`, `r`, `R`, `d`, `q`, `x`, `e`, and `Y` when grid navigation is active; run actions require an active SQL console |
| Editor `Space` or `\` (`EditorLeader`) | `f`, `y`, `Y`, `d`, `?`, plus editor-owned `r`, `R`, `n`, `s`, `q`, `x`, `e`, and `t` transaction sequences |
| `Ctrl-w` (pane prefix) | `h/j/k/l` direction, `Ctrl-w` clockwise focus cycle, `f` maximize, `=`, and pane resize operators |
| `g` | `g` first item, `t`/`T` next/previous tab |
| `z` in Explorer | `z`/`t`/`b` middle/top/bottom alignment |
| `z` in grid | `z`/`t`/`b` middle/top/bottom row alignment |
| `[` / `]` | `t` previous/next tab aliases |
| Relation `d` | `d` delete current row |
| Relation `y` | `y` yank current row |
| Record View `g` | `g` first field |

## Explorer

## Database Dashboard

Open the connection dashboard with `Space b` from Explorer or Results. The
dashboard is read-only in the current release and does not issue cancel or
terminate commands.

| Keys | Behavior |
| --- | --- |
| `Space b` | Open or focus the connection dashboard |
| `1` / `2` / `3` | Overview / Processes / Charts |
| `r` | Refresh metrics and the visible process list |
| `p` | Pause or resume dashboard polling |
| Printable text / `Backspace` / `Ctrl-U` on Processes | Filter visible processes |

The process list is bounded and may be incomplete when database privileges do
not expose every session. Filtering applies to the bounded visible snapshot.

| Keys | Behavior |
| --- | --- |
| `j/k`, Up/Down | Move selection |
| `gg`, `Home` | First visible node |
| `G`, `End` | Last visible node |
| `H/M/L` | Top/middle/bottom visible node |
| `Ctrl-f/Ctrl-b` | Page down/up |
| `Ctrl-d/Ctrl-u` | Half-page down/up |
| `zz/zt/zb` | Align selection |
| `h/l`, Left/Right | Collapse/expand |
| `o` | Toggle expansion |
| `Enter` | Open/activate selected node |
| `/` | Open visible-node find |
| `f` | Open catalog search |
| `n/N` | Next/previous confirmed find/search match |
| `n` | New profile |
| `e` | Edit selected profile |
| `a` | Add a supported object when the selected PostgreSQL node has a create option |
| `e` | Edit the directly selected PostgreSQL catalog object when supported |
| `c/x` | Connect/disconnect selected profile |
| `d` | Request profile deletion |
| `r` | Refresh catalog |
| `p` | Open relation Data preview |
| `D` | Open relation DDL |
| `s` | Open Profile Access |
| `y` | Copy selected node name |

Visible-node find is Editing until Enter. While Editing, printable keys,
Backspace, Ctrl-U, Enter, and Esc belong to find input. After confirmation,
`n`, `N`, and Esc cycle or close it. Catalog search Editing accepts text,
Backspace, Ctrl-U, navigation, Enter to locate, and Esc; after confirmation,
`n`, `N`, and Esc are the active controls.

The Explorer `a` and catalog-object `e` rows are capability-aware. They are
not shown for unsupported drivers, synthetic/status rows, or objects without
an adapter-provided create/edit capability. The Catalog Editor picker, form,
preview, and busy pages expose their own contextual rows; busy `Esc` dismisses
the editor and late responses are rejected by request identity and stale checks.

## Catalog Editor

`j/k` selects an object type in the picker, `Enter` chooses it, `Enter` previews
a form mutation, and `Enter` applies the SQL preview. `Esc` cancels the picker
or form, returns from preview, or cancels a busy editor.

## SQL Editor

### Normal

| Keys | Behavior |
| --- | --- |
| `h/j/k/l`, arrows | Move cursor |
| `i` | Insert at cursor |
| `a` | Insert after cursor |
| `o` | Open line below |
| `x`, Delete | Delete character |
| `u` / `Ctrl-r` | Undo/redo |
| `F5` | Run current statement |
| `Shift-F5` | Run complete buffer |
| `Space f` | Format current/selected SQL |
| `Space y` | Copy current statement |
| `Space Y` | Copy complete buffer |
| `Space d` | Choose execution target |
| `Space r` / `Space R` | Run current/all SQL through EditorLeader |
| `Space tt/tc/tr` | Toggle, commit, or roll back transaction through EditorLeader |
| `g` then `g/t/T` | First item or next/previous tab |
| `[` then `t`, `]` then `t` | Tab aliases |
| `?` | Open contextual Help |
| `Space ?` | Open editor Help through EditorLeader |

### Insert and Replace

| Keys | Behavior |
| --- | --- |
| Printable text, Enter, Tab | Insert/edit text |
| arrows, Home, End | Move cursor |
| Backspace, Delete | Delete text |
| `Ctrl-w/U/H` | Delete previous word, to line start, or backspace |
| `Ctrl-Z` | Undo the focused editor text edit |
| `Ctrl-Shift-Z` | Redo the focused editor text edit |
| `Ctrl-Space` | Trigger completion |
| `Esc` | Return to Normal |
| `F5` | Run SQL |

`Ctrl-w` in Insert/Replace is an editor text command, never the pane prefix.
`Space` remains text input in Insert/Replace; it does not start either Leader.

### Non-Vim Text Inputs

All focused non-Vim text inputs use the same editing contract, including profile
fields, catalog forms, WHERE/ORDER BY filters, cell editing, console search and
rename, Help search, Explorer search, Dashboard process filtering, notification
search, and confirmation inputs.

| Keys | Behavior |
| --- | --- |
| `Ctrl-Z` | Undo the focused input's latest edit group |
| `Ctrl-Shift-Z` | Redo the focused input's latest undone edit group |
| `Ctrl-Shift-Z` or `Ctrl-Z` with shifted `Z` | Alternate terminal encoding of redo |

Consecutive typed characters and consecutive deletions are grouped into one
edit. Paste, completion replacement, word deletion, line deletion, and clear
are atomic edits. Moving to another field closes the current edit group. A new
edit after undo discards the redo branch. History is local to the focused field
and is not persisted.

### Visual

| Keys | Behavior |
| --- | --- |
| `y` | Copy selection |
| `Esc` | Return to Normal |
| `F5` | Run selection |
| `Space f` | Format selection |
| `Space y/Y` | Copy selection/buffer through EditorLeader |

Visual Char, Visual Line, and Visual Block are presented as the Editor Visual
context. Empty selections do not fall back to the whole buffer. `Space ?` is
handled by EditorLeader in this context as editor Help; plain `?` is handled by
the application mapper as contextual Help.

## SQL Results Data

| Keys | Behavior |
| --- | --- |
| `h/j/k/l`, arrows | Move selected cell |
| `gg`, `G` | First/last row |
| `H/M/L` | Top/middle/bottom visible row |
| `Ctrl-d/u`, `Ctrl-f/b` | Half-page/page movement |
| `zz/zt/zb` | Align selected row |
| `v` | Open Record View when data exists |
| `y` | Copy selected cell |
| `Y` | Copy selected row as TSV |
| Application `Space Y` | Copy row with headers when grid navigation is active |
| `o` | Switch to Output |
| `0` / `^` | Select first column |
| `$` | Select last column |
| `/` / `s` | Focus WHERE/ORDER BY when Data Query is available |

## SQL Output and Plan

Output and Plan are read-only Vim text views, not grids.

| Keys | Behavior |
| --- | --- |
| `j/k`, arrows | Move through text |
| `gg/G` | First/last text position |
| `H/M/L` | Viewport alignment |
| `Ctrl-d/u`, `Ctrl-f/b` | Half-page/page movement |
| `/` | Search text |
| `v/V`, `Ctrl-v` | Visual Char/Line/Block selection |
| `y` | Copy selected text |
| `o` | Return to Data |

Output `o` is the `Output o` view toggle and is handled before the read-only editor mapper. Other read-only Vim
editing commands remain disabled. Output hints describe text, never cells.

## Relation Data

The catalog contexts are `RelationDataBrowse`, `RelationDataEdit`,
`RelationDataVisual`, and `RelationDataBusy`.

### Browse

| Keys | Behavior |
| --- | --- |
| `h/j/k/l`, arrows | Move through cells |
| `gg/G`, `H/M/L`, page controls | Move rows and viewport |
| `yy` | Yank current row |
| `Y` | Copy current row as TSV |
| `dd` | Delete current row after the real pending sequence |
| `e` | Edit selected cell when editing is available |
| `a` | Insert row when editing is available |
| `V` | Enter Visual Line when editing is available |
| `p` | Paste row when editing is available |
| `u` | Undo row changes when editing is available |
| `Ctrl-s` | Commit changes when editing is available |
| `Ctrl-r` | Redo row changes when editing is available |
| `Ctrl-x` | Roll back relation changes when editing is available |
| `/` / `s` | Focus WHERE/ORDER BY when query capability is available |
| `r` | Refresh relation |

The Relation Browse context uses `yy`/`yank row`, not SQL Results `y`/copy cell.
`Enter`, `[` and `]` are not executable Relation Data bindings in the current
mapper and are not documented as actions.

### EditCell

| Keys | Behavior |
| --- | --- |
| `Enter` | Apply cell edit |
| `Esc` | Cancel cell edit |
| Printable text, Backspace, Delete | Edit the cell value |
| `Ctrl-w`, `Ctrl-u`, `Ctrl-h` | Delete word, to start, or backspace |

### Visual Line

| Keys | Behavior |
| --- | --- |
| `j/k` | Extend selected rows |
| `y` | Yank selected rows |
| `d` | Delete selected rows |
| `V` | Cancel Visual Line |

### Busy

Busy relation requests do not advertise browse, edit, paste, yank, or delete
controls. `p` returns to Data, `r` refreshes the relation, and `Ctrl-c` remains

## Relation DDL

Relation DDL is a read-only Vim text view.

| Keys | Behavior |
| --- | --- |
| `j/k`, arrows | Move through DDL |
| `gg/G`, `H/M/L` | Move to ends or align viewport |
| `Ctrl-d/u`, `Ctrl-f/b` | Scroll viewport |
| `/` | Search DDL |
| `v/V`, `Ctrl-v` | Select DDL text |
| `y` | Copy selected text |
| `p` | Return to Data |
| `r` | Refresh relation |

## Record View

| Keys | Behavior |
| --- | --- |
| `j/k`, Up/Down | Move through fields |
| `h/l`, Left/Right | Move through records |
| `gg`, `Home` | First field |
| `G`, `End` | Last field |
| `Esc`, `q`, `v` | Close Record View |

Record View is read-only and is only available when the active result has at
least one row and one column.

## Data Query Inputs and Completion

Data Query input is available only in a Data view with a valid SQL or Relation
query capability. It replaces the underlying grid hints.

| Keys | Behavior |
| --- | --- |
| Printable text, Backspace, Delete | Edit WHERE/ORDER BY text |
| `Ctrl-w/U/H` | Text editing controls |
| `Tab`, `Shift-Tab` | Switch WHERE and ORDER BY |
| `Enter` | Submit query |
| `Esc` | Cancel input |
| `Ctrl-n`, `Ctrl-p` | Next/previous completion when completion is open |
| `Tab` or `Enter` | Accept completion when completion is open |
| `Esc` | Dismiss completion when completion is open |

## Profile Manager

### Form

| Keys | Behavior |
| --- | --- |
| `Tab`, `Shift-Tab`, BackTab | Move fields |
| `j/k`, Up/Down | Move fields; literal in text fields where applicable |
| Left/Right, `h/l` | Cycle compatible driver/options |
| Enter/Space | Activate option or action |
| `F5` | Test connection |
| `Ctrl-s` | Save |
| `Ctrl-Enter` | Save and connect |
| `Esc` | Close manager |

### Scope

| Keys | Behavior |
| --- | --- |
| `j/k`, Up/Down | Move scope rows |
| `Space` | Toggle selected scope row when not loading |
| `r` | Refresh discovery when not loading |
| `Esc`, Enter | Return to Form |

### Delete and Loading

Profile delete confirmation uses `Enter`/`y` to confirm and `Esc`/`n`/`q` to
cancel. During scope discovery loading, `Space` and `r` are unavailable; return
and navigation remain available.

## SQL Editor List

| Keys | Behavior |
| --- | --- |
| Printable text, Backspace | Filter editors |
| `j/k`, Up/Down | Move selection |
| `Enter` | Activate selected editor |
| `Esc` | Close list |

## Help Search

Help captures its display context and capabilities when opened. Later pane/tab
changes do not change the list. Printable text and paste edit the search query;
Backspace removes a character; Ctrl-U clears; Up/Down moves selection; Enter
executes the selected executable shortcut; Esc closes Help.

Display-only catalog rows are informational and cannot execute an action.

## Profile Access

| Keys | Behavior |
| --- | --- |
| `j/k`, Up/Down | Move access choice |
| `Enter` | Apply choice |
| `Esc`, `q` | Cancel |

## Message

| Keys | Behavior |
| --- | --- |
| `Esc`, `q` | Close message |

## Confirmations and Selectors

### Substitute Confirmation

`y` accepts one replacement, `n` rejects one, `a` accepts all, `l` accepts the
last and stops, and `Esc`/`q` quits substitution.

### Execution Confirmation

`Enter`/`e`/`y` executes; `Esc`/`n`/`q` cancels; Tab/Left/Right changes the
focused choice.

### Manual Cancellation

`Enter`/`c` confirms cancellation and rollback; `Esc`/`k` keeps the query
running; Tab/Left/Right changes the choice.

### Transaction Exit

`a` abandons, `r` rolls back, `c` commits, and Enter confirms the selected
choice. Esc/n cancels; Tab/Left/Right changes the choice.

### Clear Transaction Outcome

Enter/y clears the unknown outcome. Esc/n/q cancels.

### Target Selector

`j/k` or Up/Down moves between targets; Enter confirms; Esc cancels.

### Delete SQL Editor

Enter confirms permanent deletion; Esc cancels.

### Page Size Selector

`j/k` or Up/Down moves between page sizes. Enter applies the selected size; Esc
cancels.

### Catalog Drop Confirmation

Type the confirmation text, use Backspace or Ctrl-U to edit it, Enter confirms
the drop, and Esc cancels. Printable `y` and `Y` are text input, not shortcuts.

## Mouse

Left click focuses panels, activates tabs, selects Explorer rows and grid cells,
and toggles view selectors. Wheel scrolling affects the panel under the pointer.
Relation DDL scrolls vertically; Relation Data scrolls the grid. Mouse/paste/
resize/focus events clear pending key sequences. Use `--mouse off` for terminal
 native selection behavior.

## Update Center

`F9` opens the Update Center from normal workspace contexts. `Enter` confirms the
focused action, `Tab` or Left/Right changes focus, and `Esc`/`q` closes it. `r`
retries a failed check. A native update is installed without interrupting the
current session; `Restart now` activates it after running SQL, relation edits, and
manual transactions have been safely resolved. `Later` keeps the current process
running.
