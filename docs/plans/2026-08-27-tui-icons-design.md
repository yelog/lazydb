# TUI Icon System Design

**Status:** Approved on 2026-08-27

## Goal

Replace LazyDB's mixed and partly incorrect hard-coded glyphs with a coherent icon system that shows recognizable PostgreSQL, MySQL, and SQLite brands, gives every catalog object a stable semantic icon, and remains usable in terminals without Nerd Fonts.

## Decisions

- Support three explicit modes: `nerd-font`, `unicode`, and `ascii`.
- Default to `nerd-font` because visual fidelity is the primary goal.
- Expose the mode through the process-local `--icons` CLI option.
- Add the same option to `lazydb.nvim` so plugin users do not need to bypass its argv builder.
- Do not modify the connection profile file or introduce a general application configuration migration.
- Do not attempt terminal font detection; terminals do not expose reliable glyph coverage information.
- Do not use terminal image protocols for Explorer tree icons.

## Architecture

Create `src/ui/icons.rs` as the sole owner of UI icon mappings. It defines a Clap-compatible `IconMode` and a small `IconSet` value that maps `DatabaseKind` and `CatalogKind` to static strings.

The startup and rendering flow is:

```text
CLI --icons
  -> Cli.icons: IconMode
  -> IconSet::new(cli.icons)
  -> ui::render_with_state(..., icons)
  -> render_explorer(..., icons)
  -> icons.database(kind) / icons.catalog(kind)
```

Icon preferences remain outside `App`. They are presentation settings, not database, reducer, workspace, or persisted profile state. Runtime switching is deliberately deferred.

The simple `ui::render` entry point continues to use `IconSet::default()` so existing callers retain the default Nerd Font behavior. The stateful runtime and tests can pass an explicit icon set.

## Icon Sources

The Nerd Font mode uses the `nerd-font-symbols` crate rather than embedding private-use characters directly in source.

- Devicons supplies recognizable PostgreSQL, MySQL, and SQLite brand glyphs.
- Material Design Icons supplies database object glyphs such as database, table, table column, key, function, and trigger-like symbols.
- Where no exact catalog concept exists, choose the closest unambiguous MDI glyph and lock that choice in tests.

The current hard-coded connection glyphs are incorrect: they resolve to JSON, generic database, and chip icons rather than PostgreSQL, MySQL, and SQLite. They are replaced with `DEV_POSTGRESQL`, `DEV_MYSQL`, and `DEV_SQLITE`.

Unicode mode uses standard Unicode geometric and box-drawing symbols and avoids emoji and private-use characters. ASCII mode uses short labels such as `PG`, `MY`, `SQ`, `DB`, `SC`, `TB`, `PK`, and `FK`. Rendering must not assume an icon is exactly one terminal cell wide.

## CLI And Plugin

Add the global CLI option:

```text
--icons nerd-font|unicode|ascii
```

The default is `nerd-font`. Invalid values are rejected by Clap before terminal initialization. This is an additive CLI change, so `CLI_API_VERSION` remains unchanged and `capabilities --json` does not gain a feature flag.

Add `icons = nil` to `lazydb.nvim` defaults. Accepted non-nil values are the same three strings. The plugin appends `--icons VALUE` to its stable argv list.

## Rendering And Layout

Only Explorer connection and catalog prefixes change. Existing theme colors continue to style each complete glyph or label. Explorer rows retain the existing `"{icon} "` separator, which supports both one-cell Nerd Font glyphs and two-character ASCII labels.

No fixed x-coordinate may be derived from a one-cell icon assumption. Existing clipping remains responsible for narrow panes. Tests cover an ASCII render at narrow and normal widths to prevent prefix changes from corrupting adjacent text.

## Error Handling And Compatibility

- Unsupported CLI or plugin values fail with an actionable validation error.
- Missing Nerd Font glyphs cannot be detected reliably; documentation directs users to `--icons unicode` or `--icons ascii`.
- SSH rendering depends on the local terminal font, not the remote host's fonts.
- Nerd Fonts 3.x or a compatible Symbols Nerd Font fallback is documented as recommended, not required.
- No font files, SVGs, images, or logo assets are distributed by LazyDB.

## Testing

Unit tests in `src/ui/icons.rs` cover every `DatabaseKind` and `CatalogKind` in all three modes. They verify that mappings are non-empty and control-character-free, Unicode mappings do not contain private-use code points, and ASCII mappings contain only ASCII.

CLI tests verify the default, all valid values, and invalid-value rejection. Ratatui `TestBackend` tests verify that the selected mode reaches connection and catalog rows and that ASCII prefixes render safely in constrained widths. Neovim plugin tests verify validation and argv construction.

Final verification runs formatting, focused tests, the full Rust suite, Clippy with warnings denied, and the plugin test suite. Manual acceptance checks compare Nerd Font and fallback modes in representative terminals.

## Documentation

Update `README.md`, `docs/configuration.md`, and `lazydb.nvim/README.md` with:

- the three modes and the default;
- Nerd Fonts 3.x recommendation;
- CLI and plugin configuration examples;
- fallback guidance for boxes or misaligned glyphs;
- the fact that `--icons` applies only to the current process.

## Deferred Work

- Persisting the icon mode in a future general application configuration file.
- Runtime mode switching.
- Automatic terminal font or glyph probing.
- `ratatui-image` and terminal image protocols for large logos.
- Shipping or downloading fonts from LazyDB.
