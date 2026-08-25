# Keybindings

This document lists the operational keyboard contract. The in-app footer shows
the shortest relevant subset; `F1` works everywhere and `?` is backward search in
Editor Normal mode but contextual help outside the editor.

## Global

| Key | Action |
| --- | --- |
| `F1` | Contextual help |
| `?` | Contextual help outside Insert mode |
| `Ctrl-w h` | Focus Explorer |
| `Ctrl-w j` | Focus Results |
| `Ctrl-w k`, `Ctrl-w l` | Focus Editor |
| `Tab`, `Shift-Tab` | Next/previous panel outside Insert mode |
| `[t`, `]t` | Previous/next LazyDB tab |
| `Space n` | New SQL console |
| `Ctrl-c` | Cancel active query; otherwise leave Insert mode |
| `Q` | Quit LazyDB in Normal mode |
| `Space c` | Open the connection Profile Manager |

## Profile Manager

| Key | Action |
| --- | --- |
| `j/k`, arrows | Select a profile or field |
| `Enter` | Connect selected profile, or edit the selected form field |
| `n` | Create a new profile |
| `t` | Test Connection without saving |
| `s` | Save the profile without connecting |
| `c` | Save & Connect |
| `d` | Delete after confirmation |
| `Space` | Toggle checkboxes and SQLite memory mode |
| `Esc` | Close, cancel, or leave the manager |

## Explorer

| Key | Action |
| --- | --- |
| `j/k`, arrows | Move selection |
| `Home/End` | First/last visible object |
| `h/l`, left/right, `Enter` | Collapse or expand |
| `r` | Reload catalog |
| `p` | Open a 500-row table/view preview |
| `D` | Open available object DDL in a new SQL tab |

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

## Results

| Key | Action |
| --- | --- |
| `h/j/k/l`, arrows | Move selected cell |
| `o` | Switch Data/Output |

## Mouse

- Left click switches panels, activates tabs, selects catalog rows, selects result
  cells, and toggles Data/Output.
- Wheel scroll moves the panel under the pointer.
- Closing the Neovim floating window hides it without stopping LazyDB.
