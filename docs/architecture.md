# Architecture

LazyDB is currently a modular Rust monolith plus an independent thin Neovim
wrapper.

```text
Crossterm input / DB events
           |
         Action
           |
      App::update
           |
        Command
           |
        Runtime
           |
DatabaseConnection enum
  |          |          |
Postgres   MySQL      SQLite
```

## Reducer Boundary

`App::update` is the only application-state mutation path. It accepts semantic
actions and returns commands. It does not access the terminal, filesystem, Tokio,
or SQLx. This makes tabs, focus, editor behavior, and stale-event rejection
deterministic unit-test targets.

The runtime executes commands and emits actions through a Tokio channel. A
connection generation prevents a late connection result from replacing a newer
connection. Each console has its own generation so a cancelled or old query
cannot overwrite a newer run.

## Profile and Credential Boundary

`ProfileStore` atomically persists versioned connection metadata in TOML without
passwords. `Runtime` owns profile CRUD, native keyring operations, session-only
secrets, and connection identity validation. Native keyring calls run in
blocking tasks; references use `keyring:dev.lazydb.lazydb/<profile-uuid>`.
`App` applies profile state only after matching runtime completion actions, so a
failed save or switch can compensate without exposing a password.

## Database Boundary

`DatabaseConnection` dispatches to concrete `PostgresAdapter`, `MySqlAdapter`, or
`SqliteAdapter`. This is intentionally not SQLx `AnyPool`: native catalog, type,
SSL, DDL, cancellation, and transaction behavior must remain visible.

Dynamic user SQL consumes SQLx `raw_sql().fetch_many()` so statement result
markers are retained. Each adapter decodes its concrete row type into owned
`CellValue` values. NULL, empty text, and empty binary remain distinct. Unknown
types become `Unsupported` values rather than failing the complete query.

## Rendering Boundary

The UI is immediate-mode Ratatui. `AppLayout` selects too-small, focus, standard,
or wide composition. `UiState` owns mouse hit regions and transient TachyonFX
effects; effects never enter application state. Inactive effects cause no idle
redraw.

Database text passes through terminal-control sanitization before it reaches
diagnostic state. Future clipboard/export paths must retain the same boundary.

## Neovim Boundary

`lazydb.nvim` owns only a scratch terminal buffer, floating window, argv/cwd,
one process per Neovim tab handle, and lifecycle/health checks. It does not parse
SQL, connect to databases, or store credentials. The embedded LazyDB TUI
provides the same Profile Manager and keyring behavior when launched by Neovim.

## Next Architectural Steps

- Persist console manifests and SQL files atomically.
- Pin physical connections for manual transaction sessions.
- Replace complete result collection with bounded row batches and viewport
  storage.
- Add database-native cancellation and connection quarantine.
- Add statement selection, parser-backed risk classification, and completion.
- Add a conservative mutation planner for stable-key single-table previews.
