# TUI Icon System Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace LazyDB's incorrect and mixed hard-coded glyphs with configurable Nerd Font, Unicode, and ASCII icon sets for database connections and catalog objects.

**Architecture:** Keep icon selection outside application and persistence state. Parse a process-local `IconMode` with Clap, construct an immutable `IconSet` in `run_tui`, and pass it through the Ratatui render path to the Explorer; centralize every semantic mapping in `src/ui/icons.rs` and expose the same option through `lazydb.nvim`.

**Tech Stack:** Rust 2024, Clap 4.5, Ratatui 0.30, Crossterm 0.29, `nerd-font-symbols` 0.3, Lua, Neovim 0.10, Cargo test, rustfmt, Clippy

---

### Task 1: Add the Typed Icon Sets

**Files:**
- Modify: `Cargo.toml:10-37`
- Modify: `Cargo.lock`
- Create: `src/ui/icons.rs`
- Modify: `src/ui/mod.rs:1-20`
- Test: `src/ui/icons.rs`

**Step 1: Add failing icon mapping tests**

Create `src/ui/icons.rs` with the public types and a `#[cfg(test)]` module before adding mappings:

```rust
use clap::ValueEnum;

use crate::{db::catalog::CatalogKind, profile::DatabaseKind};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum IconMode {
    #[default]
    NerdFont,
    Unicode,
    Ascii,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IconSet {
    mode: IconMode,
}

impl IconSet {
    pub const fn new(mode: IconMode) -> Self {
        Self { mode }
    }

    pub const fn database(self, kind: DatabaseKind) -> &'static str {
        todo!()
    }

    pub const fn catalog(self, kind: CatalogKind) -> &'static str {
        todo!()
    }
}
```

Add table-driven tests with these exhaustive enum lists:

```rust
const DATABASE_KINDS: [DatabaseKind; 3] = [
    DatabaseKind::Postgres,
    DatabaseKind::MySql,
    DatabaseKind::Sqlite,
];

const CATALOG_KINDS: [CatalogKind; 16] = [
    CatalogKind::Database,
    CatalogKind::Schema,
    CatalogKind::Table,
    CatalogKind::View,
    CatalogKind::MaterializedView,
    CatalogKind::Column,
    CatalogKind::Index,
    CatalogKind::PrimaryKey,
    CatalogKind::UniqueConstraint,
    CatalogKind::ForeignKey,
    CatalogKind::CheckConstraint,
    CatalogKind::Function,
    CatalogKind::Procedure,
    CatalogKind::Trigger,
    CatalogKind::Sequence,
    CatalogKind::Type,
];
```

Cover these contracts:

- Every mapping is non-empty and contains no control character.
- Every `Ascii` mapping satisfies `str::is_ascii`.
- Every `Unicode` mapping excludes Unicode private-use ranges `U+E000..=U+F8FF`, `U+F0000..=U+FFFFD`, and `U+100000..=U+10FFFD`.
- Brand mappings are exactly `DEV_POSTGRESQL`, `DEV_MYSQL`, and `DEV_SQLITE`.
- Representative catalog mappings are exactly `MD_DATABASE`, `MD_DATABASE_OUTLINE`, `MD_TABLE`, `MD_TABLE_COLUMN`, `MD_KEY`, and `MD_FUNCTION`.

Use a small test-only helper for the private-use check; do not add a production Unicode classification abstraction.

**Step 2: Register the module and verify the tests fail**

Add to `src/ui/mod.rs`:

```rust
pub mod icons;
```

Run:

```bash
cargo test ui::icons --all-features
```

Expected: compilation fails because `nerd_font_symbols` is not yet a dependency and the mapping methods are unimplemented.

**Step 3: Add the dependency**

Add to `[dependencies]` in `Cargo.toml`:

```toml
nerd-font-symbols = "0.3"
```

Run `cargo check --all-features` once to resolve the dependency and update `Cargo.lock`.

**Step 4: Implement the complete mappings**

Import the named constants rather than embedding private-use glyphs:

```rust
use nerd_font_symbols::{dev, md};
```

Implement the following semantic matrix:

| Kind | Nerd Font | Unicode | ASCII |
| --- | --- | --- | --- |
| PostgreSQL | `dev::DEV_POSTGRESQL` | `PG` | `PG` |
| MySQL | `dev::DEV_MYSQL` | `MY` | `MY` |
| SQLite | `dev::DEV_SQLITE` | `SQ` | `SQ` |
| Database | `md::MD_DATABASE` | `◆` | `DB` |
| Schema | `md::MD_DATABASE_OUTLINE` | `◇` | `SC` |
| Table | `md::MD_TABLE` | `▦` | `TB` |
| View | `md::MD_TABLE_EYE` | `◈` | `VW` |
| Materialized view | `md::MD_TABLE_SYNC` | `◉` | `MV` |
| Column | `md::MD_TABLE_COLUMN` | `│` | `CL` |
| Index | `md::MD_FORMAT_LIST_NUMBERED` | `#` | `IX` |
| Primary key | `md::MD_KEY` | `●` | `PK` |
| Unique constraint | `md::MD_KEY_STAR` | `○` | `UQ` |
| Foreign key | `md::MD_KEY_LINK` | `↗` | `FK` |
| Check constraint | `md::MD_CHECK_DECAGRAM` | `✓` | `CK` |
| Function | `md::MD_FUNCTION` | `ƒ` | `FN` |
| Procedure | `md::MD_CODE_BRACES` | `λ` | `PR` |
| Trigger | `md::MD_LIGHTNING_BOLT` | `!` | `TG` |
| Sequence | `md::MD_ORDER_NUMERIC_ASCENDING` | `≡` | `SQ` |
| Type | `md::MD_SHAPE_OUTLINE` | `τ` | `TY` |

Before accepting the table, compile against `nerd-font-symbols` 0.3. If an MDI constant in the proposed matrix does not exist in that version, search the crate's generated `md` module and choose the closest existing constant with the same semantics. Update both the implementation and exact-constant test; do not fall back to a hard-coded PUA character.

Keep the match exhaustive so adding a future `DatabaseKind` or `CatalogKind` causes a compile failure until it receives an icon.

**Step 5: Run focused tests**

Run:

```bash
cargo test ui::icons --all-features
```

Expected: all icon mapping and character-safety tests pass.

**Step 6: Commit the icon model**

```bash
git add Cargo.toml Cargo.lock src/ui/icons.rs src/ui/mod.rs
git commit -m "feat(ui): add configurable icon sets"
```

### Task 2: Parse the Icon Mode and Inject It Into Rendering

**Files:**
- Modify: `src/cli.rs:3-39`
- Modify: `src/cli.rs:190-254`
- Modify: `src/runtime.rs:1667-1754`
- Modify: `src/ui/mod.rs:159-248`
- Modify: `src/ui/mod.rs:500-555`
- Delete from: `src/ui/mod.rs:1497-1522`
- Modify: `tests/mouse.rs`
- Modify: `tests/ui_render.rs:82-102`
- Test: `src/cli.rs`

**Step 1: Add failing CLI parsing tests**

In the existing `src/cli.rs` test module, parse minimal argument arrays and assert:

```rust
assert_eq!(
    Cli::try_parse_from(["lazydb"]).unwrap().icons,
    IconMode::NerdFont,
);
assert_eq!(
    Cli::try_parse_from(["lazydb", "--icons", "unicode"])
        .unwrap()
        .icons,
    IconMode::Unicode,
);
assert_eq!(
    Cli::try_parse_from(["lazydb", "--icons", "ascii"])
        .unwrap()
        .icons,
    IconMode::Ascii,
);
assert!(Cli::try_parse_from(["lazydb", "--icons", "emoji"]).is_err());
```

Also test explicit `--icons nerd-font` so Clap's kebab-case spelling is locked down.

**Step 2: Run the CLI tests and verify they fail**

Run:

```bash
cargo test cli::tests --all-features
```

Expected: compilation fails because `Cli` has no `icons` field.

**Step 3: Add the global CLI option**

Import `crate::ui::icons::IconMode` and add next to the existing display-related options:

```rust
#[arg(long, global = true, value_enum, default_value_t = IconMode::NerdFont)]
pub icons: IconMode,
```

Do not modify `CLI_API_VERSION` or the capabilities feature list.

**Step 4: Change the render boundary**

Import `IconSet` into `src/ui/mod.rs`. Preserve the convenience renderer with the default:

```rust
pub fn render(frame: &mut Frame<'_>, app: &App) {
    let mut state = UiState::new(true);
    render_with_state(frame, app, &mut state, IconSet::default());
}

pub fn render_with_state(
    frame: &mut Frame<'_>,
    app: &App,
    state: &mut UiState,
    icons: IconSet,
) {
    // existing body
}
```

Add `icons: IconSet` to `render_explorer` and replace the existing calls:

```rust
let icon = visible.kind.map_or("·", |kind| icons.catalog(kind));
```

```rust
format!("{} ", icons.database(kind))
```

Remove `catalog_icon` and `database_icon` entirely. Do not leave aliases around for compatibility; they have no external consumers and contain the incorrect brand mappings.

**Step 5: Inject one immutable set from the runtime**

In `run_tui`, construct the set before entering the draw loop:

```rust
let icons = IconSet::new(cli.icons);
```

Pass the copyable value to both draw closures:

```rust
terminal.draw(|frame| ui::render_with_state(frame, &app, &mut ui_state, icons))?;
```

Do not place `IconSet` in `App`, `Runtime`, `UiState`, or persisted workspace data.

**Step 6: Update existing test render calls**

Update direct `ui::render_with_state` calls in `tests/mouse.rs` to pass `IconSet::default()`.

In `tests/ui_render.rs`, extend the local helper rather than changing every test:

```rust
fn render_with_state(app: &App, width: u16, height: u16) -> (String, UiState) {
    render_with_icons(app, width, height, IconSet::default())
}

fn render_with_icons(
    app: &App,
    width: u16,
    height: u16,
    icons: IconSet,
) -> (String, UiState) {
    // existing TestBackend setup
    terminal
        .draw(|frame| ui::render_with_state(frame, app, &mut state, icons))
        .unwrap();
    // existing buffer extraction
}
```

Update the few direct calls later in `tests/ui_render.rs` to pass `IconSet::default()`.

**Step 7: Run CLI, UI, and mouse tests**

Run:

```bash
cargo test cli::tests --all-features
cargo test --test ui_render --all-features
cargo test --test mouse --all-features
```

Expected: all tests compile and pass; existing snapshots/assertions retain default Nerd Font behavior except assertions that encoded the previous incorrect glyphs, which must be updated to named `IconSet` expectations.

**Step 8: Commit the runtime integration**

```bash
git add src/cli.rs src/runtime.rs src/ui/mod.rs tests/mouse.rs tests/ui_render.rs
git commit -m "feat(cli): select explorer icon mode"
```

### Task 3: Prove Each Mode Reaches Explorer Rows

**Files:**
- Modify: `tests/ui_render.rs:82-198`
- Reference: `src/ui/mod.rs:500-555`

**Step 1: Add a reusable catalog rendering fixture**

Extract or add a small helper in `tests/ui_render.rs` that creates an `App` with:

- one connected PostgreSQL profile;
- one expanded `Database` entry;
- one expanded `Schema` entry;
- one visible `Table` entry named `users`.

Use `CatalogEntry` constructors and `rebuild_projection`; do not bypass normalized catalog validation. Keep the fixture local to `tests/ui_render.rs`.

**Step 2: Add a failing mode propagation test**

Render the same app through `render_with_icons` in all three modes and assert representative prefixes:

```rust
let nerd = render_with_icons(&app, 120, 36, IconSet::new(IconMode::NerdFont)).0;
assert!(nerd.contains(nerd_font_symbols::dev::DEV_POSTGRESQL));
assert!(nerd.contains(nerd_font_symbols::md::MD_TABLE));

let unicode = render_with_icons(&app, 120, 36, IconSet::new(IconMode::Unicode)).0;
assert!(unicode.contains("PG "));
assert!(unicode.contains("▦ users"));

let ascii = render_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii)).0;
assert!(ascii.contains("PG "));
assert!(ascii.contains("TB users"));
```

Use the actual profile and catalog row text emitted by the fixture when writing the complete assertions. Avoid asserting an entire terminal buffer.

**Step 3: Run the exact test**

Run:

```bash
cargo test --test ui_render explorer_uses_selected_icon_mode --all-features
```

Expected: PASS if Task 2 propagated `IconSet` to both connection-root and catalog-node rendering branches. If it fails, use the assertion to locate the missed branch before continuing; do not weaken the expected prefixes.

**Step 4: Fix any missed Explorer branches minimally**

If loading, group, empty, or error rows still use punctuation, leave them unchanged unless they represent a `DatabaseKind` or `CatalogKind`. The scope is semantic database/catalog icons, not every status marker in the UI.

Ensure color selection still comes from `database_color` and `catalog_color`; only the displayed symbol changes.

**Step 5: Add constrained-width coverage**

Render the ASCII fixture at the smallest width that still shows the Explorer rather than the global `TooSmall` view. Assert:

- rendering does not panic;
- the selected row text remains present or is cleanly clipped;
- no icon is concatenated directly with its label (`"TBusers"` must not occur);
- hit regions remain within terminal bounds.

Derive the width from the existing layout tests instead of introducing a duplicate layout threshold constant.

**Step 6: Run focused rendering tests**

Run:

```bash
cargo test --test ui_render explorer_uses_selected_icon_mode --all-features
cargo test --test ui_render ascii_icons_render_safely_in_narrow_explorer --all-features
cargo test --test ui_render --all-features
```

Expected: all UI render tests pass.

**Step 7: Commit render regressions**

```bash
git add tests/ui_render.rs
git commit -m "test(ui): cover explorer icon modes"
```

### Task 4: Expose Icon Selection Through lazydb.nvim

**Files:**
- Modify: `lazydb.nvim/lua/lazydb/config.lua:3-17`
- Modify: `lazydb.nvim/lua/lazydb/config.lua:27-61`
- Modify: `lazydb.nvim/lua/lazydb/config.lua:74-94`
- Modify: `lazydb.nvim/tests/lazydb_spec.lua:116-145`
- Test: `lazydb.nvim/tests/lazydb_spec.lua`

**Step 1: Add failing plugin configuration tests**

Extend `merges configuration and builds a stable argv list` with:

```lua
icons = "unicode",
```

Assert the stable argv list includes:

```lua
"--icons",
"unicode",
```

Add validation coverage asserting that `icons = "emoji"` raises an error mentioning the accepted values.

**Step 2: Run the plugin suite and verify it fails**

Run:

```bash
nvim --headless -u lazydb.nvim/tests/minimal_init.lua \
  -c "lua require('lazydb_spec').run()" -c qa
```

Expected: the argv assertion fails because `config.argv()` ignores `icons`, and the invalid value is accepted.

**Step 3: Add and validate the plugin option**

Add to defaults:

```lua
icons = nil,
```

Validate non-nil values:

```lua
if options.icons ~= nil
    and not vim.tbl_contains({ "nerd-font", "unicode", "ascii" }, options.icons) then
  error("lazydb.nvim: icons must be 'nerd-font', 'unicode', or 'ascii'", 3)
end
```

Append to argv after the color option:

```lua
if values.icons then
  vim.list_extend(argv, { "--icons", values.icons })
end
```

Preserve `nil` so the CLI owns the default rather than duplicating `nerd-font` in the plugin process arguments.

**Step 4: Run the plugin suite**

Run the same headless Neovim command.

Expected: all plugin tests pass and argv remains an argument list rather than a shell command.

**Step 5: Commit the plugin integration**

```bash
git add lazydb.nvim/lua/lazydb/config.lua lazydb.nvim/tests/lazydb_spec.lua
git commit -m "feat(nvim): configure icon mode"
```

### Task 5: Document Font Requirements and Complete Verification

**Files:**
- Modify: `README.md:53-63`
- Modify: `README.md:114-127`
- Modify: `docs/configuration.md:1-19`
- Modify: `lazydb.nvim/README.md:12-37`
- Reference: `docs/plans/2026-08-27-tui-icons-design.md`

**Step 1: Update the root requirements and CLI contract**

In `README.md`, keep UTF-8 as the only hard terminal requirement and add Nerd Fonts 3.x as recommended for branded icons. Explain:

```text
--icons nerd-font   # default; branded database and object glyphs
--icons unicode     # standard Unicode fallback
--icons ascii       # maximum compatibility
```

Add `[--icons nerd-font|unicode|ascii]` to the CLI contract block. State that boxes or misaligned glyphs indicate an incompatible font and that the user should select a fallback mode.

**Step 2: Document process-local configuration**

Add an `Icon Mode` section to `docs/configuration.md` that states:

- the default is `nerd-font`;
- `--icons` applies only to the current invocation;
- it does not alter `connections.toml`;
- Nerd Fonts 3.x or Symbols Nerd Font Mono fallback is recommended;
- SSH uses the font configured by the local terminal;
- LazyDB does not auto-detect or distribute fonts.

Do not describe `--config` as storing this option; the current file is still a connection profile store.

**Step 3: Document the Neovim option**

Add to the setup example in `lazydb.nvim/README.md`:

```lua
icons = nil, -- "nerd-font", "unicode", or "ascii"; nil uses the CLI default
```

Mention that configuration changes affect newly started or restarted sessions, matching existing plugin behavior.

**Step 4: Run formatting and focused verification**

Run:

```bash
cargo fmt --check
cargo test ui::icons --all-features
cargo test cli::tests --all-features
cargo test --test ui_render --all-features
cargo test --test mouse --all-features
nvim --headless -u lazydb.nvim/tests/minimal_init.lua \
  -c "lua require('lazydb_spec').run()" -c qa
```

Expected: every command exits successfully.

**Step 5: Run full project verification**

Run:

```bash
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: the full suite passes and Clippy reports no warnings.

**Step 6: Perform manual terminal acceptance checks**

Using a Nerd Font 3.x terminal, run:

```bash
cargo run -- --icons nerd-font --url sqlite::memory:
```

Verify the SQLite connection uses the recognizable Devicons SQLite glyph and Database, Schema, and Table rows use distinct MDI glyphs with aligned labels.

Using a terminal profile without Nerd Font fallback, run:

```bash
cargo run -- --icons unicode --url sqlite::memory:
cargo run -- --icons ascii --url sqlite::memory:
```

Verify there are no replacement boxes, rows remain aligned, labels are separated from prefixes, and narrow Explorer rendering clips cleanly. If PostgreSQL and MySQL profiles are available, inspect their connection roots; no live server is required to verify an offline root's brand prefix.

**Step 7: Commit documentation**

```bash
git add README.md docs/configuration.md lazydb.nvim/README.md \
  docs/plans/2026-08-27-tui-icons-design.md \
  docs/plans/2026-08-27-tui-icons-implementation.md
git commit -m "docs: explain terminal icon modes"
```

**Step 8: Inspect the final diff**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only the icon dependency, icon module, CLI/render integration, tests, plugin configuration, and documentation are changed; `git diff --check` prints no errors.
