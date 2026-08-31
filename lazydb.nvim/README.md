# lazydb.nvim

Thin Neovim terminal integration for LazyDB. The plugin is published as the
`lazydb.nvim/` subdirectory of the [`yelog/lazydb`](https://github.com/yelog/lazydb)
repository. It starts the LazyDB CLI directly in a floating terminal, with one
process per Neovim tab. It does not connect to databases or interpret SQL.

## Requirements

- Neovim 0.10 or newer
- A `lazydb` executable implementing CLI API 1

## Installation

Install the `lazydb` CLI first. The plugin is not a replacement for the CLI.

### lazy.nvim

This repository is a monorepo, so point `lazy.nvim` at the checked-out plugin
subdirectory:

```bash
git clone https://github.com/yelog/lazydb.git ~/src/lazydb
```

```lua
return {
  {
    dir = vim.fn.expand("~/src/lazydb/lazydb.nvim"),
    name = "lazydb.nvim",
    cmd = { "LazyDB", "LazyDBToggle", "LazyDBRestart" },
    keys = {
      {
        "<leader>db",
        function()
          require("lazydb").toggle()
        end,
        desc = "Toggle LazyDB",
      },
    },
    opts = {
      executable = "lazydb",
      window = { width = 0.92, height = 0.90, border = "rounded" },
    },
  },
}
```

`lazy.nvim` will lazy-load this plugin when one of the declared commands or
keys is used. The `dir` value must point to `lazydb.nvim/`, not the repository
root. This is required because `yelog/lazydb` is a monorepo rather than a
standalone Neovim plugin repository.

### Native `pack/*/start/*`

Neovim 0.10 and newer loads packages from `pack/*/start/*`. When using the
monorepo checkout, add the nested plugin directory explicitly:

```bash
mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/lazydb/start"
git clone https://github.com/yelog/lazydb.git \
  "${XDG_DATA_HOME:-$HOME/.local/share}/nvim/site/pack/lazydb/start/lazydb-repository"
```

```lua
vim.opt.runtimepath:append(
  vim.fn.expand("~/.local/share/nvim/site/pack/lazydb/start/lazydb-repository/lazydb.nvim")
)
require("lazydb").setup({ executable = "lazydb" })
```

For a standalone plugin checkout, place its contents directly at:

```text
~/.local/share/nvim/site/pack/lazydb/start/lazydb.nvim/
```

### `vim.pack.add()`

Neovim 0.12 and newer can install Git repositories with `vim.pack.add()`. The
current repository is a monorepo, so the nested plugin directory still needs
to be added to `runtimepath`:

```lua
vim.pack.add({ "https://github.com/yelog/lazydb.git" }, { confirm = true })
vim.opt.runtimepath:append(
  vim.fn.stdpath("data") .. "/site/pack/core/opt/lazydb/lazydb.nvim"
)
vim.cmd("packadd! lazydb")
require("lazydb").setup({ executable = "lazydb" })
```

If your Neovim version stores the clone under a different package name, use
that directory in the `runtimepath` expression. The important part is that the
path ends in `lazydb.nvim`, where `lua/lazydb/` and `plugin/lazydb.lua` live.

For Neovim 0.10 and 0.11, use the native `pack/*/start/*` approach above.

## Setup

After the plugin is available on `runtimepath`, configure it if the executable
is not on `PATH` or if you need CLI options:

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
