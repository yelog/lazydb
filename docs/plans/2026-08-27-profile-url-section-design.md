# Profile URL Section Design

## Goal

Separate structured connection fields from direct URL configuration in the new/edit connection form. Remove the visible URL Format selector, keep URL synchronization behavior unchanged, and place a fixed URL section with driver-specific examples below the structured fields.

## Existing Behavior

The form currently exposes Driver, URL Format, and URL before the structured fields. `ConnectionUrlFormat` is not merely presentational: URL parsing records the submitted format and structured edits use it to regenerate an equivalent URL. URL editing is committed when focus leaves the URL field, with parse errors retaining URL focus.

## Design

### Preserve Internal URL Format State

Keep `ConnectionUrlFormat`, `ProfileDraft::url_format`, URL parsing, and URL regeneration. Remove `ProfileField::UrlFormat` only from the visible field arrays and navigation order. The internal format continues to update when a URL is parsed and continues to control `refresh_url`.

### Split the Form Into Two Configuration Areas

The upper area contains structured fields and scrolls independently. It excludes both `UrlFormat` and `Url`.

The lower area is fixed above messages and buttons and contains:

1. One blank separator row.
2. The existing editable URL field.
3. Driver-specific read-only example rows.

The URL field continues to use the existing rendering, cursor, horizontal scrolling, hit target, commit, validation, and sensitive-data handling paths.

### URL Examples

Examples depend only on the current Driver and use static placeholders, never current profile values.

Postgres:

```text
postgres://user:password@host:5432/database
postgresql://user:password@host:5432/database
jdbc:postgresql://host:5432/database
```

MySQL:

```text
mysql://user:password@host:3306/database
jdbc:mysql://host:3306/database
```

SQLite:

```text
sqlite:///path/to/database.db
file:/path/to/database.db
jdbc:sqlite:/path/to/database.db
```

Examples use muted or dim styling, do not enter keyboard navigation, and do not create hit regions.

### Compact Layout

The URL input remains visible at every supported form size. Example rows consume only remaining space: normal layouts show all examples; compact layouts show at least one. Extra examples are removed before URL input, messages, buttons, or hints are removed.

The structured field viewport uses the height left after reserving the fixed URL section. When URL is selected, the structured viewport does not scroll because URL is outside it.

### Navigation and Data Flow

Visible field order becomes structured fields, URL, then action buttons. Tab, BackTab, and directional navigation use this order naturally after the field arrays are reordered.

Leaving URL still invokes `commit_url`. Successful parsing still updates Driver and structured values. Invalid URLs still retain URL focus and display the existing error. Structured edits still call `refresh_url` using the retained internal URL format.

## Alternatives Rejected

### Put URL at the End of the Existing Scrollable List

This is simpler but makes URL and examples disappear in compact layouts and does not reliably communicate two alternative configuration methods.

### Delete ConnectionUrlFormat Entirely

This would break format preservation and alter URL regeneration behavior, exceeding the requested presentation-only change.

### Always Reserve Every Example Row

This would over-compress structured fields on compact terminals. Adaptive example count keeps the primary input and controls usable.

## Verification

Tests should verify that URL Format is absent from rendering and navigation, URL remains available at the bottom with a blank separator, examples follow the current Driver, compact layouts retain URL and at least one example, examples have no hit regions, and all existing URL import, commit, validation, regeneration, and secret-redaction tests continue to pass.
