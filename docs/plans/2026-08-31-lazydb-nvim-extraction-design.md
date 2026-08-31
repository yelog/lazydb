# lazydb.nvim Repository Extraction Design

## Goal

Move the Neovim integration from the `lazydb.nvim/` subdirectory of
`yelog/lazydb` into a standalone public `yelog/lazydb.nvim` repository whose
repository root is a valid Neovim plugin root.

## Current Boundary

The plugin is already operationally independent from the Rust application. It
owns only terminal integration and communicates with the installed `lazydb`
binary through a stable CLI API:

- `lazydb capabilities --json` reports CLI API compatibility;
- normal sessions launch the executable with an argv array;
- the plugin never connects to databases or handles credentials;
- one LazyDB process is maintained per Neovim tab.

Its public Lua API is `setup`, `open`, `toggle`, `hide`, `stop`, `restart`, and
`status`. It registers `:LazyDB`, `:LazyDBToggle`, `:LazyDBHide`, `:LazyDBStop`,
and `:LazyDBRestart`. Session cleanup covers window close, buffer wipe, tab
close, process exit, restart, and Neovim exit. The health check validates
Neovim, terminal jobs, locale, clipboard, executable availability, and CLI API
1 without connecting to a database.

## Source Ownership

`yelog/lazydb.nvim` becomes the only source repository for the plugin. The main
`yelog/lazydb` repository removes its `lazydb.nvim/` directory after the
standalone repository has been created, tested, and pushed.

The extraction uses the current tested plugin snapshot as the standalone
repository's initial commit. The standalone README records that the plugin was
extracted from `yelog/lazydb`, and the main repository's existing Git history
continues to preserve all earlier plugin changes.

No bidirectional sync or generated mirror is introduced. This avoids duplicate
sources of truth and synchronization credentials.

## Standalone Layout

The new repository root contains:

```text
lazydb.nvim/
├── .github/workflows/ci.yml
├── doc/lazydb.txt
├── lua/lazydb/config.lua
├── lua/lazydb/health.lua
├── lua/lazydb/init.lua
├── plugin/lazydb.lua
├── tests/lazydb_spec.lua
├── tests/minimal_init.lua
├── .gitignore
├── LICENSE-APACHE
├── LICENSE-MIT
└── README.md
```

The root directly exposes `lua/`, `plugin/`, and `doc/`, allowing standard
plugin-manager specifications such as `{ "yelog/lazydb.nvim" }` without local
directory or runtimepath workarounds.

## Installation Documentation

The standalone README makes `lazy.nvim` the primary installation example and
also documents:

- native `pack/*/start/*` installation;
- Neovim 0.12 `vim.pack.add()`;
- manual git clone;
- LazyDB CLI installation as a separate prerequisite;
- every supported plugin option and user command;
- `:checkhealth lazydb`.

The main repository README keeps a concise Neovim integration section linking
to `yelog/lazydb.nvim` and uses standard remote plugin-manager examples.

## Compatibility Contract

The standalone plugin requires Neovim 0.10 or newer and a `lazydb` executable
implementing CLI API 1. The plugin version is independent from the CLI semantic
version. Compatibility is determined by `cli_api`, not matching Git tags.

The plugin repository does not bundle, download, or auto-update the CLI. This
keeps executable installation and editor integration independently auditable.

## CI

The standalone repository runs the existing headless suite against:

- Neovim 0.10.4, the minimum supported version;
- the current stable Neovim release provided by the setup action.

The test command is:

```sh
nvim --headless -u tests/minimal_init.lua \
  -c "lua require('lazydb_spec').run()" -c qa
```

The main repository removes its Neovim plugin job because plugin source and
tests no longer live there. Its Rust CI and release workflows remain unchanged.

## Licensing

The standalone repository retains the main project's dual license:

- MIT;
- Apache-2.0.

Both license files are copied into the standalone repository. Repository
metadata identifies the project as a Neovim frontend for LazyDB and links back
to `yelog/lazydb` for the CLI.

## Migration Sequence

1. Populate `/Users/yelog/workspace/vi/lazydb.nvim` from the tested plugin
   snapshot.
2. Add standalone metadata, CI, README, and licenses.
3. Run the headless plugin suite from the standalone root.
4. Initialize Git and create the initial commit.
5. Create the public GitHub repository `yelog/lazydb.nvim` and push `main`.
6. Verify the remote repository and Actions workflow.
7. Update the main repository README and CI references.
8. Delete the main repository `lazydb.nvim/` directory.
9. Run main-repository Rust checks and standalone plugin checks again.
10. Commit and push the main-repository migration.

The standalone repository is pushed before deleting the embedded source, so a
failed GitHub operation cannot leave the plugin unavailable.

## Acceptance Criteria

- `https://github.com/yelog/lazydb.nvim` exists and is public.
- The local repository exists at `/Users/yelog/workspace/vi/lazydb.nvim`.
- `{ "yelog/lazydb.nvim" }` is a valid `lazy.nvim` plugin specification.
- All eight existing plugin tests pass from the standalone root.
- The standalone repository contains both project licenses and independent CI.
- The main repository no longer contains `lazydb.nvim/`.
- The main README links to and configures the standalone plugin repository.
- Main-repository Rust CI and Release workflows do not reference removed plugin
  files.
