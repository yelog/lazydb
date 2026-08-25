# Keybindings

This document lists bindings that are operational in M0. The in-app footer shows
the shortest relevant subset; `F1` works everywhere and `?` works outside editor
Insert mode.

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
| `Q` | Quit LazyDB |

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
| `F5`, `Space r` | Execute the complete buffer |

Insert mode:

| Key | Action |
| --- | --- |
| `Esc`, idle `Ctrl-c` | Return to Normal mode |
| `Tab` | Insert a tab character |
| arrows, Home, End | Move cursor |
| Backspace, Delete, Enter | Edit text |

M0 intentionally executes the complete buffer. Current statement and visual
selection execution will be added only with reliable statement-boundary support.

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
