# Profile Driver Icons Design

## Goal

Show a database icon beside every Postgres, MySQL, and SQLite option in the Driver selector on the new/edit connection form, using the same icon scheme as Explorer root nodes.

## Existing Behavior

The Driver selector in `src/ui/profiles.rs` renders the three database names as plain text. Explorer root nodes already use `IconSet::database(DatabaseKind)`, which provides Nerd Font icons and safe Unicode/ASCII fallbacks.

## Design

Reuse the existing `IconSet::database` mapping rather than adding a second Driver-specific mapping. Pass the active `IconSet` through the profile manager rendering path to `render_driver_options`.

Each option will render as one label containing its icon and name:

```text
<database icon> POSTGRES  <database icon> MYSQL  <database icon> SQLITE
```

The option width will be calculated from the complete rendered label with `CellWidth`, including the icon. The existing one-cell gap between options remains. The hit region will cover the complete icon-and-name label so keyboard and mouse interaction retain the same semantics.

The selected, busy, and unselected styles remain unchanged. The active icon mode controls the output, so Nerd Font mode uses database brand icons while Unicode and ASCII modes continue using `PG`, `MY`, and `SQ` fallbacks.

## Scope

- Modify only the profile UI rendering path and its tests.
- Do not change `DatabaseKind`, `IconSet`, Explorer rendering, persistence, or input actions.
- Preserve truncation behavior when the available selector area is too narrow: stop rendering options that do not fit completely.

## Verification

UI tests will verify that all three Driver options render the expected database icons in the default icon mode and that the selector continues to expose individual hit regions with correct selected styling. Existing tests for alternate icon modes and profile form rendering must continue to pass.
