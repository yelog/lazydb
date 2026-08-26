# Connection Management Optimization Design

**Status:** Approved

**Date:** 2026-08-26

## Summary

LazyDB will make credential requirements explicit, render connection drivers as
horizontal choices, expose PostgreSQL's default schema, derive Explorer scope
from that schema until the user customizes visibility, and add a safe connection
URL editing view that stays synchronized with structured connection settings.

Structured connection settings remain the only persisted source of truth. Raw
URLs and passwords are never persisted.

## Credential Policy

`ConnectionProfile` replaces the ambiguous optional secret reference with an
explicit credential policy:

- `None`: the profile is intentionally passwordless.
- `Prompt`: the profile requires a password, but it is only retained for the
  current process.
- `Keyring`: the password is stored in the native credential store.

Password resolution checks the current session, the startup password bound to
the selected profile, and then the persisted policy. A `Prompt` profile without
a current session password emits `CredentialsRequired` instead of attempting an
unauthenticated connection.

Profile persistence moves to version 3. Version 2 keyring references migrate to
`Keyring`; profiles without a reference migrate to `None` because their prior
session-only intent cannot be recovered safely.

## Profile Form

The DRIVER row renders PostgreSQL, MySQL, and SQLite horizontally. The selected
driver remains highlighted even when the row does not have focus.

Horizontal keys edit choices:

- `h` and Left choose the previous driver or enum value.
- `l` and Right choose the next driver or enum value.

Vertical keys move through fields:

- `j`, Down, and Tab move to the next field.
- `k`, Up, and Shift-Tab move to the previous field.

Text fields retain literal `h`, `j`, `k`, and `l` input because the form has no
separate navigation/edit mode. Up and Down provide vertical navigation while
Left and Right move the text cursor.

Mouse regions select a driver directly instead of cycling relative to the
current value.

## Schema And Catalog Scope

PostgreSQL profiles expose a DEFAULT SCHEMA field. MySQL does not expose an
independent schema because its database is the schema. SQLite continues to use
`main` by default.

`default_schema` controls the default execution namespace and completion
ranking. `catalog_scope` remains the sole visibility policy for Explorer,
metadata, completion candidates, and relation operations.

Drafts track whether catalog scope is derived or explicitly customized. A
derived scope is regenerated when driver, database, or default schema changes.
Once the user changes Visible Objects, the explicit scope is preserved and
validated instead of silently overwritten.

Filtering remains at the adapter query and CatalogPage validation layers. The
Explorer renderer does not add a second visibility policy.

## Connection URL

Profiles persist a URL format preference, not a raw URL. Supported forms are:

- PostgreSQL: `postgres://`, `postgresql://`, `jdbc:postgresql://`.
- MySQL: `mysql://`, `jdbc:mysql://`.
- SQLite: `sqlite:`, `file:`, `jdbc:sqlite:`.

The form stores URL input in a redacted secret-backed input. Parsing and
formatting are pure operations around structured connection settings.

URL-to-fields synchronization is explicit and atomic. It occurs when the URL is
submitted, loses focus, or before Test/Save. Parse failure leaves the existing
structured fields unchanged.

Field-to-URL synchronization only formats the latest structured values and
never invokes the parser, preventing feedback loops. Temporarily invalid fields
retain the last valid URL until validation succeeds.

Passwords parsed from URLs move immediately into the draft password secret and
are removed from the visible canonical URL. A URL without a password preserves
the current session or keyring credential. Existing keyring passwords are never
loaded into the URL and no `***` placeholder is generated.

The first implementation supports only recognized connection parameters and
rejects unknown parameters rather than silently discarding them. This keeps the
adapter contract predictable and avoids prematurely adding arbitrary property
persistence.

## Error Handling

- Missing session credentials open the existing profile editor focused on
  Password.
- URL errors never include the raw URL.
- URL parsing applies no partial draft updates.
- Native keyring unavailability downgrades remembered credentials to `Prompt`
  and displays a warning.
- Explicit catalog scopes that exclude the default schema fail validation.
- Debug output, actions, profile TOML, and user-facing URLs never contain a
  password.

## Testing

Tests cover profile v2-to-v3 migration, session-only restart behavior,
remembered-keyring reload, passwordless profiles, driver rendering and keymaps,
derived versus explicit catalog scope, parser/formatter round trips, encoded
credentials, URL synchronization, and secret redaction.

The final verification runs formatting, focused tests, the full Rust test suite,
and Clippy with warnings denied.
