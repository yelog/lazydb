# LazyDB M0 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a polished, runnable LazyDB foundation with a responsive Ratatui shell, safe profile import, real PostgreSQL/MySQL/SQLite connection and basic query paths, a base catalog explorer, multiple SQL tabs, contextual help, and a thin Neovim wrapper.

**Architecture:** Implement a single Rust crate with one-way reducer state updates and concrete database adapters behind an application-owned enum/contract. Keep database work outside the render loop and return generation-tagged events over bounded channels. Ship the Neovim integration as a separate Lua directory that only owns terminal process lifecycle.

**Tech Stack:** Rust 1.94+, Ratatui 0.30, Crossterm 0.29, TachyonFX 0.25, Tokio 1, SQLx 0.9 with concrete PostgreSQL/MySQL/SQLite drivers, Clap 4, Serde/TOML, sqlparser-rs, ratatui-textarea, Lua for Neovim 0.10+.

---

## Execution Notes

- The workspace had no Git repository when this plan was written. Commit steps
  are intentionally omitted; do not initialize or commit without explicit user
  approval.
- Follow TDD for pure domain code and reducer behavior. For the terminal shell,
  write TestBackend assertions before wiring the live Crossterm backend.
- No M0 menu entry may invoke a placeholder. Keep later actions out of the menu.
- PostgreSQL/MySQL tests are opt-in through environment variables; SQLite tests
  must run everywhere.

### Task 1: Bootstrap the Rust Project and Stable CLI Contract

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/cli.rs`
- Create: `.gitignore`
- Create: `LICENSE-MIT`
- Create: `LICENSE-APACHE`

**Step 1: Write CLI parsing tests**

Add tests to `src/cli.rs` for:

```rust
#[test]
fn parses_direct_connection_url() {
    let cli = Cli::try_parse_from([
        "lazydb",
        "--url",
        "sqlite://demo.db",
        "--read-only",
    ])
    .unwrap();
    assert_eq!(cli.url.as_deref(), Some("sqlite://demo.db"));
    assert!(cli.read_only);
}

#[test]
fn parses_machine_readable_capabilities() {
    let cli = Cli::try_parse_from(["lazydb", "capabilities", "--json"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Capabilities { json: true })
    ));
}
```

**Step 2: Run the focused tests and observe failure**

Run: `cargo +stable test cli::tests -- --nocapture`

Expected: compilation fails because the crate and CLI types do not exist.

**Step 3: Add minimal project metadata and CLI types**

Set package name `lazydb`, edition `2024`, `rust-version = "1.94"`, and license
`MIT OR Apache-2.0`. Define:

```rust
#[derive(Debug, Parser)]
#[command(name = "lazydb", version, about)]
pub struct Cli {
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub profile: Option<String>,
    #[arg(long, hide = true)]
    pub url: Option<String>,
    #[arg(long)]
    pub read_only: bool,
    #[arg(long, value_enum, default_value_t = MouseMode::Auto)]
    pub mouse: MouseMode,
    #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,
    #[command(subcommand)]
    pub command: Option<Command>,
}
```

Subcommands are `version --json`, `capabilities --json`, and
`doctor --json --profile <name>`.

**Step 4: Implement and verify JSON contracts**

`lazydb capabilities --json` must include:

```json
{
  "version": "0.1.0",
  "cli_api": 1,
  "features": ["mouse", "read-only", "context-help"],
  "drivers": ["postgres", "mysql", "sqlite"]
}
```

Run:

```text
cargo +stable test cli::tests
cargo +stable run -- capabilities --json
cargo +stable run -- version --json
```

Expected: tests pass and both commands emit one valid JSON object.

### Task 2: Parse Connection URLs Without Persisting Secrets

**Files:**
- Create: `src/profile.rs`
- Create: `src/security.rs`
- Modify: `src/lib.rs`

**Step 1: Write profile import tests**

Cover these inputs:

```text
jdbc:postgresql://10.196.178.221:30345/moss?currentSchema=tools
postgres://alice:secret@db.example.com:5433/app?sslmode=require
jdbc:mysql://db.example.com:3307/catalog?useSSL=true
mysql://alice:secret@db.example.com/catalog
sqlite:///tmp/lazydb.db
file:/tmp/lazydb.db
:memory:
```

Assertions must verify kind, host, port defaults, user, database/path,
`currentSchema`, SSL intent, and that serialization/debug output never contains
`secret`.

**Step 2: Run and observe failure**

Run: `cargo +stable test profile::tests security::tests`

Expected: unresolved modules/types.

**Step 3: Implement normalized profile models**

Create:

```rust
pub enum DatabaseKind { Postgres, MySql, Sqlite }

pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub kind: DatabaseKind,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub default_schema: Option<String>,
    pub sqlite_path: Option<PathBuf>,
    pub ssl_mode: SslMode,
    pub secret_ref: Option<String>,
    pub read_only: bool,
    pub environment: Environment,
}

pub struct ImportedProfile {
    pub profile: ConnectionProfile,
    pub transient_password: Option<SecretString>,
}
```

Strip a leading `jdbc:` before URL parsing. Parse `file:` and `:memory:` as
SQLite special forms. Implement a custom redacted `Debug` for imported secrets.

**Step 4: Add central text/DSN sanitization**

Implement:

```rust
pub fn sanitize_terminal_text(value: &str) -> String;
pub fn redact_connection_string(value: &str) -> String;
```

Remove or visibly escape C0 controls other than newline/tab, ESC, CSI, OSC, and
DEL. Replace URL passwords with `***`.

**Step 5: Verify**

Run: `cargo +stable test profile::tests security::tests`

Expected: all parsing, redaction, and control-sequence tests pass.

### Task 3: Persist Safe Profiles and Workspace Metadata

**Files:**
- Create: `src/persistence/mod.rs`
- Create: `src/persistence/paths.rs`
- Create: `src/persistence/profiles.rs`
- Modify: `src/lib.rs`
- Test: `tests/persistence.rs`

**Step 1: Write temporary-directory tests**

Verify:

- Saving and loading multiple profiles preserves stable IDs and order.
- Serialized TOML contains `secret_ref` but no transient password.
- A malformed file returns a typed error and does not overwrite the source.
- Atomic save leaves no temporary file after success.

Use `tempfile::TempDir`; do not touch the user's actual config directory.

**Step 2: Run and observe failure**

Run: `cargo +stable test --test persistence`

Expected: persistence module is missing.

**Step 3: Implement platform paths and repositories**

Use `directories::ProjectDirs` with qualifier `dev`, organization `lazydb`, and
application `lazydb`. Expose an override for tests and `--config`.

```rust
pub struct ProfileStore { path: PathBuf }

impl ProfileStore {
    pub fn load(&self) -> Result<Vec<ConnectionProfile>, PersistenceError>;
    pub fn save(&self, profiles: &[ConnectionProfile])
        -> Result<(), PersistenceError>;
}
```

Write to a sibling temporary file, flush, and rename. Create private directories
where supported. M0 stores only secret references; OS keyring retrieval is added
behind a `SecretStore` contract, not invoked by tests.

**Step 4: Verify**

Run: `cargo +stable test --test persistence`

Expected: all tests pass and fixture files contain no password.

### Task 4: Define Database-Neutral Result and Catalog Models

**Files:**
- Create: `src/db/mod.rs`
- Create: `src/db/catalog.rs`
- Create: `src/db/query.rs`
- Create: `src/db/value.rs`
- Modify: `src/lib.rs`

**Step 1: Write model tests**

Test that:

- Catalog object IDs include connection, native parent, kind, and native name.
- `CellValue::Null` is distinct from empty text and empty bytes.
- Long text is truncated for preview without losing original length metadata.
- Query statistics separate execution and fetch duration.

**Step 2: Run and observe failure**

Run: `cargo +stable test db::`

Expected: database modules do not exist.

**Step 3: Implement minimal models**

Use enums for catalog kinds and values. Preserve native type names and binary
content. Define capability flags explicitly rather than assuming feature parity.

```rust
pub enum CellValue {
    Null,
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Bytes(Vec<u8>),
    Unsupported { type_name: String, preview: String },
}
```

Define `DatabaseError` categories matching the design document and a
`QueryOutcome` containing columns, rows, affected rows, notices, and timing.

**Step 4: Verify**

Run: `cargo +stable test db::`

Expected: model tests pass with no SQLx dependency leaking into public models.

### Task 5: Implement SQLite Connection, Catalog, and Query Paths

**Files:**
- Create: `src/db/sqlite.rs`
- Modify: `src/db/mod.rs`
- Test: `tests/sqlite_adapter.rs`

**Step 1: Write real temporary-database tests**

Create a database with `users`, `teams`, an index, a foreign key, a view, and a
trigger. Verify:

- Probe returns SQLite version.
- Catalog returns tables/views and child columns/indexes/foreign keys.
- Query returns typed NULL, integer, real, text, and blob values.
- Non-query returns affected row count.
- Object DDL returns stored schema SQL.
- Read-only mode rejects writes at the database layer.

**Step 2: Run and observe failure**

Run: `cargo +stable test --test sqlite_adapter -- --nocapture`

Expected: SQLite adapter missing.

**Step 3: Implement the concrete adapter**

Use `SqlitePoolOptions`, explicit create/read-only options, and bounded pool
sizes. Query `sqlite_schema` and PRAGMA table-valued functions. Decode runtime
SQLite storage classes without treating declared type as the actual value type.

**Step 4: Verify**

Run: `cargo +stable test --test sqlite_adapter -- --nocapture`

Expected: all SQLite integration tests pass.

### Task 6: Implement PostgreSQL and MySQL Core Adapters

**Files:**
- Create: `src/db/postgres.rs`
- Create: `src/db/mysql.rs`
- Modify: `src/db/mod.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/mysql_adapter.rs`

**Step 1: Write dialect and opt-in integration tests**

Always-run tests verify identifier quoting and catalog SQL construction. Tests
requiring servers skip with a clear message unless these are set:

```text
LAZYDB_TEST_POSTGRES_URL
LAZYDB_TEST_MYSQL_URL
```

Server tests create isolated objects, probe version/current database, load base
catalog, run a typed query, run DML in a transaction, roll it back, and confirm no
row persisted.

**Step 2: Run and observe failure**

Run:

```text
cargo +stable test --test postgres_adapter
cargo +stable test --test mysql_adapter
```

Expected: adapters missing; after implementation, no-server runs pass with the
integration cases reported as skipped.

**Step 3: Implement concrete pools and probes**

Use `PgPoolOptions` and `MySqlPoolOptions`. Keep URL assembly inside each adapter
and retrieve secrets only at connection time. Catalog M0 scope is database/schema,
tables/views, and columns. Use `pg_catalog`/`information_schema` for PostgreSQL
and `information_schema` plus current database for MySQL.

Decode common scalar types and render unknown types as safe unsupported previews
instead of panicking. Preserve native type names.

**Step 4: Verify optional real servers**

Run the always-run tests. If Docker is available, start temporary PostgreSQL and
MySQL containers with non-production credentials, export both test URLs, rerun
the tests, then stop/remove only those named test containers.

Expected: all three adapter suites pass.

### Task 7: Build the Reducer, Tabs, Focus, and Generation Safety

**Files:**
- Create: `src/action.rs`
- Create: `src/app.rs`
- Create: `src/model/mod.rs`
- Create: `src/model/workspace.rs`
- Create: `src/model/tab.rs`
- Modify: `src/lib.rs`

**Step 1: Write reducer tests**

Cover:

- New consoles are named `console`, `console_2`, and `console_3`.
- Closing an active tab chooses a deterministic neighbor.
- Explorer/editor/results focus cycling works in both directions.
- A database result with an old owner generation is ignored.
- Query start/run/fail/cancel transitions update status and output.
- `?` opens help scoped to the focused panel; Escape closes it.

**Step 2: Run and observe failure**

Run: `cargo +stable test app::tests model::tests`

Expected: reducer/types missing.

**Step 3: Implement pure state transitions**

Keep side effects out of `App::update`. Return a vector of commands:

```rust
pub enum Command {
    Connect { profile_id: Uuid, generation: u64 },
    LoadCatalog { connection_id: Uuid, generation: u64 },
    RunQuery { tab_id: Uuid, generation: u64, sql: String },
    PersistWorkspace,
    Quit,
}
```

**Step 4: Verify**

Run: `cargo +stable test app::tests model::tests`

Expected: deterministic reducer tests pass without a terminal or database.

### Task 8: Implement the Responsive Deep-Space UI

**Files:**
- Create: `src/ui/mod.rs`
- Create: `src/ui/theme.rs`
- Create: `src/ui/layout.rs`
- Create: `src/ui/header.rs`
- Create: `src/ui/explorer.rs`
- Create: `src/ui/editor.rs`
- Create: `src/ui/results.rs`
- Create: `src/ui/footer.rs`
- Create: `src/ui/help.rs`
- Create: `src/ui/effects.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write TestBackend rendering assertions**

Render representative state at:

- 80x24: focus mode with no clipped panic.
- 120x36: Explorer plus editor/results.
- 180x50: wide layout with expanded metadata.

Assert visible product identity, connection/transaction state, active tab, panel
titles, contextual footer, and help overlay. Assert a too-small terminal displays
a resize message.

**Step 2: Run and observe failure**

Run: `cargo +stable test --test ui_render -- --nocapture`

Expected: UI module missing.

**Step 3: Implement palette and layout**

Use a graphite/blue-black base, cyan focus/success, electric blue actions, amber
pending state, and coral errors. Do not encode state only with color.

Keep all layout decisions in `ui/layout.rs` and all styles in `ui/theme.rs`.
Store rendered mouse hit regions for tabs, panels, tree rows, result subtabs, and
footer actions.

**Step 4: Add bounded effects**

Wrap TachyonFX usage in `UiEffects`. Implement focus and overlay transitions only;
idle state has no active effect. Respect `reduced_motion` and cap active redraws at
30 FPS.

**Step 5: Verify**

Run: `cargo +stable test --test ui_render -- --nocapture`

Expected: all sizes render deterministically and without panics.

### Task 9: Wire Crossterm Events, SQL Editing, and Database Commands

**Files:**
- Create: `src/event.rs`
- Create: `src/terminal.rs`
- Create: `src/input/mod.rs`
- Create: `src/input/keymap.rs`
- Create: `src/input/mouse.rs`
- Create: `src/runtime.rs`
- Modify: `src/main.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/editor.rs`
- Test: `tests/keymap.rs`

**Step 1: Write keymap sequence tests**

Verify global `?`, `Ctrl-w h/j/k/l`, `[t`/`]t`, `Space n`, `Ctrl-c`; Explorer
`h/j/k/l`, Enter, `r`, `D`, `p`; editor Normal/Insert basics and `Space r`/F5.
Ensure pending multi-key sequences time out and printable keys remain editor input
in Insert mode.

**Step 2: Run and observe failure**

Run: `cargo +stable test --test keymap`

Expected: keymap module missing.

**Step 3: Implement event and terminal guards**

Use a bounded Tokio channel for terminal, tick, resize, and database events.
Enable raw mode, alternate screen, focus events, bracketed paste, and mouse capture
according to CLI mode. Restore every mode in `Drop` and install a panic hook that
restores before printing diagnostics.

**Step 4: Wire editor and commands**

Use `ratatui-textarea` behind an app-owned editor state. M0 implements basic
Normal/Insert navigation and complete-buffer execution; current-statement and
visual selection execution remain hidden until statement selection is reliable.

Route commands through runtime tasks. A query result event includes tab ID,
generation, and sanitized database error/output data.

**Step 5: Manual smoke test**

Run:

```text
cargo +stable run -- --url sqlite::memory:
```

Expected: LazyDB enters alternate screen, renders the responsive workspace,
accepts SQL, runs a basic query, changes focus, opens help, and restores the
terminal after quit.

### Task 10: Connect Profiles, Catalog Explorer, Query Results, and Base Mouse UX

**Files:**
- Create: `src/ui/profiles.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/ui/explorer.rs`
- Modify: `src/ui/results.rs`
- Modify: `src/input/mouse.rs`
- Test: `tests/app_flow.rs`

**Step 1: Write end-to-end state flow tests**

With a temporary SQLite database, drive actions through:

1. Import a profile.
2. Test and connect.
3. Receive catalog nodes.
4. Open a table preview.
5. Receive result columns/rows.
6. Create a second console and switch tabs.
7. Open contextual help.

Also test clicking a tab, tree row, result subtab, and footer help target through
recorded hit regions.

**Step 2: Run and observe failure**

Run: `cargo +stable test --test app_flow -- --nocapture`

Expected: profile/catalog/runtime UI flow incomplete.

**Step 3: Implement only operational actions**

Provide profile list/import, connection test, connect, refresh, base object tree,
DDL-open when supported, table preview, query result grid, and Output records.
Do not expose staged editing, transaction buttons, completion, formatter, paging,
or plans until M1/M2 implements them.

**Step 4: Verify**

Run: `cargo +stable test --test app_flow -- --nocapture`

Expected: complete temporary-SQLite flow passes.

### Task 11: Implement the Thin Neovim Plugin

**Files:**
- Create: `lazydb.nvim/plugin/lazydb.lua`
- Create: `lazydb.nvim/lua/lazydb/init.lua`
- Create: `lazydb.nvim/lua/lazydb/config.lua`
- Create: `lazydb.nvim/lua/lazydb/health.lua`
- Create: `lazydb.nvim/doc/lazydb.txt`
- Create: `lazydb.nvim/README.md`
- Test: `lazydb.nvim/tests/minimal_init.lua`
- Test: `lazydb.nvim/tests/lazydb_spec.lua`

**Step 1: Write headless Lua tests**

Test configuration merge and argv construction without a shell. When possible,
use a small fake executable to verify open/toggle/hide/stop lifecycle, one session
per tab handle, and generation-safe exit cleanup.

**Step 2: Run and observe failure**

Run:

```text
nvim --headless -u lazydb.nvim/tests/minimal_init.lua \
  -c "lua require('lazydb_spec').run()" -c qa
```

Expected: plugin modules missing.

**Step 3: Implement plugin API and commands**

Expose `setup`, `open`, `toggle`, `hide`, `stop`, `restart`, and `status`.
Register `:LazyDB`, `:LazyDBToggle`, `:LazyDBHide`, `:LazyDBStop`, and
`:LazyDBRestart`. Use a scratch terminal buffer, floating window, argv list,
explicit cwd, and one session keyed by the Neovim tab handle.

Do not bind terminal-mode `q`, Escape, Ctrl-C, Ctrl-W, or Ctrl-Backslash.

**Step 4: Implement health checks**

Check Neovim version, executable lookup, `capabilities --json`, `cli_api`, UTF-8
locale, and clipboard availability. Do not connect to a database during health.

**Step 5: Verify**

Run the headless tests and manually open the built LazyDB binary in a floating
window. Expected: hiding preserves the process; stopping cleans it up.

### Task 12: Documentation and Full M0 Verification

**Files:**
- Create: `README.md`
- Create: `CONTRIBUTING.md`
- Create: `docs/architecture.md`
- Create: `docs/keybindings.md`
- Create: `docs/configuration.md`
- Create: `.github/workflows/ci.yml`

**Step 1: Document only shipped behavior**

README includes project status, install/build, direct URL examples without
passwords, screenshot placeholder instructions without claiming an image exists,
M0 capabilities, explicit limitations, and roadmap link. Document config paths,
profile security, keybindings, and Neovim setup.

**Step 2: Add CI checks**

CI runs on macOS and Linux:

```text
cargo +stable fmt --check
cargo +stable clippy --all-targets --all-features -- -D warnings
cargo +stable test --all-targets --all-features
```

Do not require PostgreSQL/MySQL service tests in the basic matrix; add a separate
Linux service job for them once the adapter tests are stable.

**Step 3: Run formatting and static checks**

Run:

```text
cargo +stable fmt --check
cargo +stable clippy --all-targets --all-features -- -D warnings
cargo +stable test --all-targets --all-features
```

Expected: all checks pass with zero warnings.

**Step 4: Run CLI and TUI smoke checks**

Run:

```text
cargo +stable run -- capabilities --json
cargo +stable run -- doctor --json
cargo +stable run -- --url sqlite::memory:
```

Expected: JSON commands are valid, doctor reports environment without leaking
secrets, and the TUI restores the terminal on every tested exit path.

**Step 5: Run Neovim checks**

Run the headless plugin suite and `:checkhealth lazydb` against the locally built
binary. Expected: compatible CLI API and clean process lifecycle.

**Step 6: Record the delivery boundary**

Update README and the final work report with three explicit lists:

- Implemented and manually exercised.
- Implemented and covered only by automated tests.
- Deferred to M1/M2, with no active UI affordance.
