# LazyDB Product and Technical Design

- Status: Approved
- Date: 2026-08-24
- Initial platforms: macOS and Linux
- License: MIT OR Apache-2.0
- Implementation language: Rust

## 1. Product Goal

LazyDB is a keyboard-first, mouse-capable terminal database IDE. Its long-term
goal is to cover the core workflows of JetBrains Database Tools/DataGrip while
remaining fast, visually distinctive, safe for production work, and usable both
as a standalone CLI and inside Neovim.

The first product version supports PostgreSQL, MySQL, and SQLite. It must provide
the same read/write core for all three databases:

- Connection profiles, URL/JDBC import, SSL options, and connection testing.
- Database object browsing and lazy metadata loading.
- Persistent, connection-scoped SQL consoles.
- Query execution, output history, tabular results, filtering, sorting, and
  paging.
- Auto-commit and manual transaction sessions.
- Conservative, staged editing of tool-generated single-table result sets.
- Vim-style navigation, discoverable contextual shortcuts, and mouse input.
- A thin Neovim terminal wrapper with one LazyDB process per Neovim tab.

The project will not claim complete DataGrip parity in its first release. Deep
schema refactoring, graphical DDL designers, procedure debuggers, arbitrary join
result editing, and broad vendor support are later product areas.

## 2. Open Source Landscape

Research performed on 2026-08-24 found no single open source project that meets
all requirements.

| Project | Strengths | Relevant gaps |
| --- | --- | --- |
| [vi-sql](https://github.com/kopecmaciej/vi-sql) | Object tree, multiple query tabs, inline editing, plans, Vim mode | One active connection and immediate writes without a robust staged transaction model |
| [lazysql](https://github.com/jorgerojas26/lazysql) | Multiple active connection pages, staged DML, atomic commit | One editor per connection, no formatter, limited plan UX |
| [sqlit](https://github.com/Maxteabag/sqlit) | Broad database support, Vim editing, formatting, explicit transactions | One active connection and editor; row actions generate SQL rather than editing directly |
| [rainfrog](https://github.com/achristmascarl/rainfrog) | Strong Rust base, transaction confirmation, query plans, Vim navigation | One active connection/editor and no direct staged grid editing |
| [Harlequin](https://github.com/tconbeer/harlequin) | Mature multiple-editor experience and adapter model | No Vim modal editor, direct row editing, or dedicated plan view |
| [dblab](https://github.com/danvergara/dblab) | Broad database connectivity and result tabs | Limited editor, transaction, and editing features |
| [gobang](https://github.com/TaKO8Ki/gobang) | Attractive Rust TUI and object browsing | Limited query/editor workflow and low recent activity |
| [termdbms](https://github.com/mathaou/termdbms) | Mouse-oriented terminal data editing | SQLite-focused and inactive |
| [vim-dadbod](https://github.com/tpope/vim-dadbod) + [vim-dadbod-ui](https://github.com/kristijanhusak/vim-dadbod-ui) | Mature native Vim/Neovim SQL workflow | Not a standalone TUI and no safe direct result editing |
| [nvim-dbee](https://github.com/kndndrj/nvim-dbee) | Native Neovim UI and multiple connections | Alpha, no staged grid editing, and GPL-3.0 licensing considerations |
| [usql](https://github.com/xo/usql), [pgcli](https://github.com/dbcli/pgcli), [mycli](https://github.com/dbcli/mycli) | Excellent REPL and completion behavior | Not full database management TUIs |

The recommended source of product patterns is a combination rather than a fork:

- LazySQL for staged changes and connection-per-workspace behavior.
- vi-sql for query tabs, object browsing, plans, and external editor behavior.
- SQLit for transaction state and formatting.
- rainfrog for dangerous statement classification and confirmation.
- lazygit.nvim for the basic terminal-wrapper concept, with stricter process
  lifecycle and argv handling.

A greenfield implementation is preferable because adapting any existing project
would require replacing its session model, mutation safety model, and visual
system at the same time.

## 3. Architecture Decision

Use a modular Rust monolith for the initial implementation. Keep one Cargo crate
until a concrete independent release or compile-time boundary requires splitting
it. Maintain strict Rust module and trait boundaries from the start.

```text
CLI / TerminalGuard
        |
App reducer <- keyboard / mouse / tick / database events
        |
Workspace + Tabs + Explorer + Editor + DataGrid + Overlays
        |
DatabaseAdapter + Catalog + QuerySession + MutationPlan
        |
PostgresAdapter | MySqlAdapter | SqliteAdapter
        |
Config + Console persistence + secret references + tracing
```

### 3.1 Technology Choices

| Area | Initial choice | Reason |
| --- | --- | --- |
| TUI | Ratatui and Crossterm | Mature immediate-mode rendering, portable input, mouse support |
| Motion | TachyonFX, isolated behind UI effects | Ratatui-native effects without coupling application state to animation state |
| Async runtime | Tokio | Database, cancellation, persistence, and event orchestration |
| Database drivers | SQLx concrete PostgreSQL, MySQL, and SQLite pools | One runtime ecosystem while preserving driver-specific metadata and behavior |
| SQL AST | sqlparser-rs | Statement classification and complete-statement analysis |
| Incremental SQL context | Tree-sitter SQL grammar | Highlighting and statement boundaries for incomplete text |
| Formatting | sqlformat behind a formatter interface | Small initial implementation with future dialect-specific replacement |
| Configuration | Serde and TOML in platform config directories | Human-readable settings and deterministic migration |
| Secrets | keyring plus secrecy wrappers | Avoid passwords in configuration, logs, and Debug output |
| Diagnostics | tracing and rolling file appender | Structured, redactable logs outside the alternate screen |
| Neovim | Lua terminal wrapper using argv-list jobs | One database/UI implementation and a small plugin maintenance surface |

Do not use SQLx `AnyPool` as the domain abstraction. The adapters use concrete
pools because catalogs, raw type information, cancellation, plans, transaction
semantics, and DDL differ by database.

### 3.2 Core Modules

```text
src/
  main.rs             CLI startup and command dispatch
  cli.rs              Stable command-line contract
  terminal.rs         Raw mode, alternate screen, panic-safe restoration
  app.rs              Reducer, commands, app lifecycle
  action.rs           Input and domain actions
  event.rs            Terminal, tick, and asynchronous event sources
  model/              Workspace, tabs, focus, results, transaction state
  input/              Keymaps, Vim mode, mouse hit testing
  ui/                 Layout, theme, components, overlays, effects
  db/
    mod.rs             Adapter contracts and capability model
    postgres.rs        PostgreSQL-specific behavior
    mysql.rs           MySQL-specific behavior
    sqlite.rs          SQLite-specific behavior
    catalog.rs         Normalized catalog types
    query.rs           Query events, values, statistics, cancellation
    mutation.rs        Staging, optimistic checks, generated DML
  sql/                 Statement selection, highlighting, completion, format
  persistence/         Profiles, consoles, settings, migrations
  security/            Secret handling, redaction, terminal-text sanitization
lazydb.nvim/           Thin Neovim plugin
tests/                 Cross-module and optional real-database tests
```

## 4. Runtime and Data Flow

LazyDB uses one-way state updates:

```text
Key / Mouse / Tick / DB Event
          -> Action
          -> App reducer
          -> Command
          -> Async task
          -> Result Event
          -> next render
```

Only the reducer mutates application state. Database and persistence tasks return
events through bounded channels. Every asynchronous operation includes a request
ID and owner generation so a late result cannot update a closed or repurposed
tab.

### 4.1 Query Sessions

Each SQL console owns:

- A stable UUID and user-visible name.
- A connection profile and default database/schema.
- Editor text, cursor, selection, undo history, and mode.
- Transaction mode and current transaction state.
- Current query job, result sets, output records, and execution statistics.

Auto-commit mode borrows a connection for an operation. Manual mode pins one
physical connection until commit, rollback, disconnect, or a failure that makes
the state unknown. A manual transaction is never moved to another pool
connection.

### 4.2 Result Streaming

Workers send rows in bounded batches. The result model enforces row, byte, and
single-cell preview budgets. The grid formats only the visible viewport plus a
small overscan. Unknown and binary values remain displayable and must never crash
the UI.

Free-form SQL is not modified by appending `LIMIT`. Tool-generated table previews
use adapter-owned paging syntax and identifier quoting. The Where and Order By
controls are clearly identified as SQL clauses and are parsed before inclusion.

### 4.3 Cancellation

- PostgreSQL: record the backend PID and cancel through a control connection.
- MySQL: record `CONNECTION_ID()` and issue `KILL QUERY` through a control
  connection.
- SQLite: use the engine interrupt/progress-handler mechanism.

Client cancellation is cooperative, but the executing connection is not returned
to the pool until a known protocol and transaction state is observed. On timeout
or an uncertain state, close the connection.

## 5. Database Adapter Contract

The common interface expresses capabilities instead of pretending that all
databases behave the same.

```rust
pub trait DatabaseAdapter: Send + Sync {
    fn kind(&self) -> DatabaseKind;
    fn capabilities(&self) -> Capabilities;
    fn quote_identifier(&self, value: &str) -> String;

    async fn test(&self, profile: &ConnectionProfile) -> Result<ServerInfo>;
    async fn connect(&self, profile: &ConnectionProfile) -> Result<Connection>;
    async fn load_catalog(
        &self,
        connection: &Connection,
        request: CatalogRequest,
    ) -> Result<CatalogPage>;
    async fn execute(
        &self,
        session: &mut QuerySession,
        request: QueryRequest,
    ) -> Result<RunningQuery>;
    async fn object_ddl(
        &self,
        connection: &Connection,
        object: ObjectId,
    ) -> Result<String>;
    fn plan_mutation(&self, request: MutationRequest) -> Result<MutationPlan>;
}
```

The actual Rust API may use boxed futures or an async-trait helper depending on
the selected stable Rust version. SQLx types do not escape adapter modules.

### 5.1 Catalog Normalization

Normalize only shared concepts:

- Database, schema, table, view, materialized view.
- Column, index, primary key, unique constraint, foreign key, check constraint.
- Routine, function, procedure, trigger, sequence, and custom type when supported.

Every node retains native identity, native kind, and an extension map. Lack of
permission is represented separately from absence. Catalog loading is lazy by
tree level and cache entries can be refreshed independently.

### 5.2 DDL

Label DDL by provenance:

- Server returned, such as MySQL `SHOW CREATE`.
- Stored normalized source, such as SQLite `sqlite_schema.sql`.
- Tool reconstructed, as required for some PostgreSQL objects.

Never present reconstructed text as exact original source.

## 6. Safe Data Editing

The first editable result surface is the tool-generated single-table preview.
Arbitrary SQL results remain read-only until origin analysis can prove they are
safe.

Editing is enabled only when:

- All editable columns originate from one real table.
- The table has a primary key or a non-null reliable unique key.
- Key and edited values have supported bind representations.
- The adapter reports transactional DML support for the target table/engine.

Edits, inserts, and deletes are staged locally. The preview records original
values, typed new values, stable row identity, and the originating table version.
Applying changes:

1. Show generated SQL, target profile, transaction boundary, and change count.
2. Start or reuse the correct transaction.
3. Execute parameterized statements in deterministic order.
4. Include stable keys and required original values in optimistic predicates.
5. Treat zero affected rows as a conflict.
6. Treat more than one affected row as an internal safety failure.
7. Roll back the complete batch on any failure.
8. Require explicit commit in manual or managed confirmation mode.

MySQL statements that can implicitly commit receive a separate warning and never
display a false rollback guarantee. Commit response loss produces `UNKNOWN`, not
automatic retry or a guessed result.

## 7. Connection Profiles and Secrets

Profiles support:

- PostgreSQL, MySQL, and SQLite.
- Host, port, user, password reference, database, default schema.
- SSL mode, CA, client certificate, client key, and hostname verification.
- Database/schema inclusion filters, defaulting to all.
- Read-only and production-environment labels.
- Standard URLs and JDBC URLs, including JDBC parameters such as
  `currentSchema`.

Imported URLs are immediately decomposed into fields and the original sensitive
string is dropped. Passwords are stored in the OS keyring. Headless systems may
use an explicit environment reference or a session-only prompt; there is no
silent plaintext fallback.

TLS identity verification is on by default. Disabling verification requires an
explicit setting and leaves a persistent warning in the workspace header.

Suggested storage:

```text
~/.config/lazydb/config.toml
~/.config/lazydb/connections.toml
~/.local/share/lazydb/workspaces/<profile-id>/manifest.json
~/.local/share/lazydb/workspaces/<profile-id>/consoles/<uuid>.sql
~/.local/state/lazydb/lazydb.log
```

Paths use platform directory APIs rather than hard-coded Unix paths. Writes are
atomic. Console text and UI state may recover after a crash; an uncommitted
database transaction never recovers or auto-replays.

## 8. Visual and Interaction Design

### 8.1 Visual Language

The approved theme is a restrained deep-space data cockpit:

- Graphite and blue-black base surfaces.
- Cool cyan focus/success color.
- Electric blue interactive color.
- Amber pending/uncertain state color.
- Coral red reserved for destructive operations and errors.
- Fine separators, focus rails, indentation, and typography instead of nested
  heavy boxes.

The visual system supports truecolor, 256-color, and monochrome fallbacks. NULL,
binary, modified, read-only, warning, and error states always have textual or
symbolic meaning in addition to color.

### 8.2 Responsive Layout

```text
+ LAZYDB - profile / database.schema - TX:AUTO - QUERY:IDLE - clock +
| EXPLORER | console_2.sql *                                           |
|          | SELECT u.id, u.name                                       |
| schema   | FROM users u                                              |
|  tables  | WHERE u.active = true;                                    |
|   users  +------------------------------------------------------------+
|          | DATA 500 rows | OUTPUT 376 ms | PLAN                      |
|          | id | name | active | created_at                           |
+----------+------------------------------------------------------------+
| NORMAL  [? help] [Space+r run] [[t/]t tabs] [Ctrl+w pane]            |
+-----------------------------------------------------------------------+
```

- Wide terminals show Explorer, editor/results, and an optional context rail.
- Medium terminals collapse the context rail and permit Explorer toggling.
- Terminals below 100 columns use a single-panel focus mode.
- Terminals too small for a safe interaction surface show a clear resize prompt.

### 8.3 Motion

Motion communicates changes rather than decorating idle screens:

- 120 ms focus sweep when changing panels.
- 160 ms dissolve for overlays.
- Low-frequency pulse while a query is running.
- Short cyan confirmation flash after commit.
- Small error displacement without prolonged flashing.

Active rendering is capped at 30 FPS. Idle screens do not redraw continuously.
`reduced_motion=true` disables nonessential effects.

### 8.4 Input

Global defaults:

| Key | Action |
| --- | --- |
| `?` | Contextual help |
| `:` | Command palette |
| `Ctrl-w h/j/k/l` | Move between panes |
| `[t`, `]t` | Previous/next LazyDB tab |
| `Space n` | New SQL console |
| `Ctrl-c` | Cancel active query |
| `Q` | Request application exit |

Explorer defaults:

| Key | Action |
| --- | --- |
| `j/k`, `gg/G` | Move and jump |
| `h/l/Enter` | Collapse, expand, or open |
| `r` | Refresh node |
| `D` | Open DDL in a new console |
| `p` | Preview table/view data |
| `/`, `n/N` | Search and repeat |

Editor defaults:

| Key | Action |
| --- | --- |
| Vim normal/insert/visual basics | Edit and select SQL |
| `Space r`, `F5` | Run selection or current statement |
| `Space R` | Run the complete buffer |
| `Space f` | Format SQL |
| `Ctrl-n/p` | Completion selection |

Grid defaults:

| Key | Action |
| --- | --- |
| `h/j/k/l`, `gg/G` | Cell/row navigation |
| `Ctrl-f/b` | Next/previous page |
| `e`, `o`, `dd` | Stage edit, insert, delete |
| `u`, `Ctrl-r` | Undo/redo staged change |
| `Space p` | Preview generated changes |
| `Space a` | Apply staged changes |

The footer always displays the most relevant actions for the active context.
Every panel opens its complete contextual shortcut overlay with `?`.

Mouse hit regions cover tree nodes, tabs, result subtabs, pagination controls,
page size, buttons, and cells. Right click opens a contextual menu. Wheel behavior
follows the panel under the pointer.

## 9. Neovim Integration

`lazydb.nvim` owns only terminal integration:

- Scratch terminal buffer and floating window.
- One process/session per Neovim tab handle.
- Open, toggle, hide, stop, restart, and status operations.
- Stable argv-list startup with explicit cwd and profile.
- Resize, process exit, tab close, buffer wipe, and Neovim exit cleanup.
- `:checkhealth lazydb` for executable, CLI API, locale, terminal, and clipboard
  checks.

The CLI owns all connections, editors, tabs, keymaps, queries, and persistence.
The plugin does not parse SQL or store credentials.

Stable CLI surface:

```text
lazydb [--config PATH] [--profile NAME] [--read-only]
       [--mouse auto|on|off] [--color auto|always|never]

lazydb version --json
lazydb capabilities --json
lazydb doctor --json [--profile NAME]
```

The plugin compares `cli_api`, not exact application versions. It does not bind
terminal-mode `q`, `Esc`, `Ctrl-c`, or `Ctrl-w`. Users return to Neovim with
`Ctrl-\\ Ctrl-n`.

## 10. Error Model

Errors are classified as configuration, authentication, network/TLS, permission,
SQL, constraint, optimistic conflict, cancellation, unknown transaction state,
or internal failure.

- The header shows a concise current state.
- The Output tab retains timestamp, phase, database error code, and useful detail.
- Remediation actions are shown where possible.
- A commit interrupted before acknowledgement becomes `UNKNOWN`; writes are
  blocked pending manual verification.
- Database text is sanitized before rendering so ANSI/OSC sequences cannot modify
  the terminal, title, or clipboard.
- Normal exit, panic, signals, and plugin termination use an RAII terminal guard.

## 11. Testing Strategy

### 11.1 Unit and Property Tests

- URL/JDBC parsing, secret removal, and redaction.
- Identifier quoting and generated paging SQL for each dialect.
- Statement boundaries and risk classification.
- Reducer, key sequences, focus, and transaction transitions.
- Mutation planning and optimistic predicates.
- Terminal control sequence sanitization.

### 11.2 TUI Tests

Use Ratatui's test backend and snapshots at 80x24, 120x36, and 180x50. Cover the
profile picker, workspace, object tree, editor, data grid, output, help, errors,
pending transactions, and color fallback modes.

### 11.3 Integration Tests

Use temporary SQLite databases and optional containerized PostgreSQL/MySQL:

- Connection and catalog loading.
- DDL retrieval and query execution.
- Auto/manual transaction commit and rollback.
- Stable-key edit, insert, delete, conflict, and constraint failure.
- Query cancellation and connection quarantine.
- SSL behavior where test infrastructure permits.

### 11.4 Neovim Tests

Headless tests verify argv handling, independent tab sessions, hide/reopen behavior,
resize, non-zero exits, buffer/tab cleanup, and generation-safe callbacks.

### 11.5 Performance and Safety Gates

- 10,000 catalog objects remain navigable through lazy loading.
- Large result sets use bounded memory and maintain responsive input.
- Completion reads only local catalog state while typing.
- Idle UI performs no continuous redraw.
- Passwords and full sensitive DSNs never occur in settings, logs, panic output,
  history, or process arguments.
- Database-provided control sequences render as inert text.

## 12. Delivery Plan

| Milestone | Scope |
| --- | --- |
| M0 runnable foundation | Rust project, responsive visual shell, action/reducer loop, profile and JDBC parsing, persistence/security foundation, three adapter skeletons with real connection/query paths, base object tree and tabs, help overlay, thin Neovim plugin |
| M1 core MVP | Deep three-database catalogs, persistent consoles, current statement/selection execution, streaming grid, filters/order/paging, output metrics, auto/manual transactions |
| M2 read/write alpha | Staged single-table editing, optimistic conflicts, commit/rollback gate, broad DDL, formatting/completion, plans, native cancellation |
| M3 1.0 hardening | Full SSL/SSH matrix, import/export, disk result spool, supported-version matrix, failure injection, performance and security hardening |

M0 contains no clickable placeholders. Features that are not operational stay out
of action menus until implemented.

## 13. Feasibility and Effort

The product is technically feasible in Rust. Ratatui can support the required
rendering and interaction. SQLx can host all three drivers in one async runtime,
but catalog, type decoding, cancellation, DDL, plans, and transaction behavior
must remain adapter-specific.

Approximate schedule for four experienced engineers, one QA/SDET, and part-time
product design:

| Phase | Increment | Cumulative |
| --- | --- | --- |
| Risk prototypes | 3-4 weeks | 3-4 weeks |
| Read-only/core MVP | 10-14 weeks | 13-18 weeks |
| Read/write alpha | 12-16 weeks | 25-34 weeks |
| Beta hardening | 12-16 weeks | 37-50 weeks |
| 1.0 release | 8-12 weeks | 45-62 weeks |

For one experienced Rust engineer, a demonstrable core takes roughly 10-16 weeks
and a hardened 1.0 is a multi-quarter effort. Estimates retain about 30 percent
uncertainty until cancellation, type decoding, large results, and multi-version
catalog prototypes are proven.

## 14. Principal Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Updating the wrong row | Critical | Default read-only, stable identity proof, parameter binding, affected-row checks |
| Unknown commit outcome | Critical | Explicit UNKNOWN state and no automatic replay |
| Cancellation contaminates a pooled connection | High | Dedicated query connection, server cancel, quarantine, close on uncertainty |
| Catalog/version drift | High | Per-driver/version fixtures, capability probing, native metadata retention |
| Large results exhaust memory | High | Bounded channels, byte budgets, viewport rendering, later disk spool |
| SQL parser disagrees with server dialect | High | Parser is a UX guard, not an authorization boundary; corpus tests and safe fallback |
| Credentials leak | Critical | Keyring, secrecy types, central redaction, secret-scanning tests |
| Terminal injection from database values | Critical | Sanitize control characters before any rendering or clipboard action |
| Visual effects harm responsiveness | Medium | Active-only animation, FPS cap, reduced-motion mode, performance tests |
| Scope expands to complete DataGrip parity | High | Milestone exit criteria based on safe user workflows, not feature count |

## 15. Primary References

- [JetBrains metadata and introspection](https://www.jetbrains.com/help/datagrip/introspection.html)
- [JetBrains code completion](https://www.jetbrains.com/help/datagrip/auto-completing-code.html)
- [JetBrains data editor](https://www.jetbrains.com/help/datagrip/data-editor-and-viewer.html)
- [JetBrains sessions](https://www.jetbrains.com/help/datagrip/managing-connection-sessions.html)
- [JetBrains query plan](https://www.jetbrains.com/help/datagrip/query-execution-plan.html)
- [JetBrains SSH and SSL](https://www.jetbrains.com/help/datagrip/configuring-ssh-and-ssl.html)
- [Ratatui](https://github.com/ratatui/ratatui)
- [TachyonFX](https://github.com/junkdog/tachyonfx)
- [SQLx](https://github.com/launchbadge/sqlx)
- [PostgreSQL system catalogs](https://www.postgresql.org/docs/current/catalogs.html)
- [PostgreSQL cancellation](https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-CANCELING-REQUESTS)
- [MySQL Information Schema](https://dev.mysql.com/doc/refman/8.4/en/information-schema.html)
- [MySQL implicit commits](https://dev.mysql.com/doc/refman/8.4/en/implicit-commit.html)
- [SQLite schema table](https://www.sqlite.org/schematab.html)
- [SQLite transactions](https://www.sqlite.org/lang_transaction.html)
- [SQLite interrupt](https://www.sqlite.org/c3ref/interrupt.html)
