# lazydb.nvim

Thin Neovim terminal integration for LazyDB. The plugin starts the LazyDB CLI
directly in a floating terminal, with one process per Neovim tab. It does not
connect to databases or interpret SQL.

## Requirements

- Neovim 0.10 or newer
- A `lazydb` executable implementing CLI API 1

## Setup

Add `lazydb.nvim` to `runtimepath`, then configure it if the executable is not on
`PATH` or if you need CLI options:

```lua
require("lazydb").setup({
  executable = "lazydb",
  cwd = nil, -- nil uses the current working directory; a function is also accepted
  config = nil,
  profile = nil,
  read_only = false,
  mouse = nil, -- "auto", "on", or "off"
  color = nil, -- "auto", "always", or "never"
  icons = nil, -- "nerd-font", "unicode", or "ascii"
  window = {
    width = 0.85,
    height = 0.80,
    border = "rounded",
    zindex = 50,
  },
})
```

The plugin passes an argv list and an explicit `cwd` to `jobstart`; it never
constructs a shell command. Configuration changes apply to sessions started or
restarted afterward.

The embedded LazyDB process owns connection profiles. On first launch with no
profiles it opens the new-profile form; `Space c` opens the manager later.
Profile metadata is stored by LazyDB, while remembered passwords use the native
OS keyring. The Neovim plugin never reads, stores, or transmits credentials.

## Commands

| Command | Action |
| --- | --- |
| `:LazyDB` | Open or reveal the session for the current tab |
| `:LazyDBToggle` | Toggle its floating window |
| `:LazyDBHide` | Hide its floating window without stopping the process |
| `:LazyDBStop` | Stop and remove the current tab's session |
| `:LazyDBRestart` | Replace the current tab's session with a new process |

The equivalent Lua functions are `open()`, `toggle()`, `hide()`, `stop()`,
`restart()`, and `status()`. `status()` returns `state`, `visible`, `tabpage`, and,
when present, `job_id`, `buffer`, `window`, and `exit_code`. States are
`starting`, `running`, `exited`, and `stopped`.

Closing the floating window only hides it. A zero exit removes the session; a
nonzero exit keeps the terminal buffer available for diagnostics until stop,
restart, buffer wipe, tab close, or Neovim exit.

No terminal-mode mappings are installed. Use Neovim's default
`Ctrl-\\ Ctrl-n` sequence to leave terminal mode.

Run `:checkhealth lazydb` to check Neovim, terminal jobs, the executable, CLI API,
locale, and clipboard support. The health check only invokes
`lazydb capabilities --json` and does not connect to a database.

## Test

```sh
nvim --headless -u lazydb.nvim/tests/minimal_init.lua \
  -c "lua require('lazydb_spec').run()" -c qa
```
