# Explorer Interaction and Visual Hierarchy Design

**Status:** Approved

**Date:** 2026-08-27

## Summary

The Explorer will separate structural navigation from object activation and
reduce redundant metadata in its tree rows. `o` will only toggle expansion,
while `Enter` will activate the selected node and open the owning relation's data
preview from a table, view, or descendant node.

Explorer rows will use semantic fields rendered as separate Ratatui spans. This
allows connection status, profile provenance, endpoint, object metadata, and
comments to have distinct hierarchy and truncation without coupling the model to
Ratatui.

## Keyboard Semantics

Explorer bindings become:

| Key | Behavior |
| --- | --- |
| `j/k`, Up/Down | Move selection |
| `l`, Right | Expand and lazily load when required |
| `h`, Left | Collapse, or move to the parent when already collapsed |
| `o` | Toggle expansion or collapse only |
| `Enter` | Activate the selected node |
| `p` | Explicitly open the owning relation's data preview |
| `D` | Open the owning relation's structure view |
| `r` | Refresh the selected catalog target |

`o` maps to the existing `ExplorerToggle` action. It must never open a relation
preview, including when a table, view, column, index, constraint, or trigger is
selected.

`Enter` continues to map to `ExplorerOpenSelected`. For a relation or any
descendant owned by a relation, it opens that relation's data preview. For a
profile, database, schema, group, status, load-more, or empty-profile row, it
executes the existing primary connect, expand, retry, pagination, or create
action.

Contextual help must describe `o` and `Enter` separately. The compact footer will
use `j/k move   o toggle   Enter open   r refresh`.

## Root Rows

Saved profile roots remove the redundant `SAVED` label. Session-only profiles
retain a low-contrast `SESSION` label because their non-persistent lifetime is
important information.

Root rows use a database-specific Nerd Font brand icon instead of the generic
middle dot. PostgreSQL, MySQL, and SQLite each receive one centralized icon
mapping. This design intentionally assumes a Nerd Font terminal; unsupported
fonts may render a replacement box.

The conceptual root layout is:

```text
▾ <database icon> connection-name  <status>  endpoint
```

For a temporary profile:

```text
▾ <database icon> connection-name  SESSION  <status>  endpoint
```

Connection name is primary text. `SESSION` and endpoint use the muted color.
Status remains independently colored. At narrow widths, preserve expansion,
database icon, and connection name first; truncate endpoint and secondary labels
before the primary identity.

## Connection Status

Stable normal states should stay quiet; transitional and error states should be
explicit. Status uses both shape and color so meaning is not color-only:

| State | Display | Color |
| --- | --- | --- |
| Online | `●` | accent |
| Offline | `○` | muted |
| Linking | `◐ CONNECTING` | warning |
| Syncing | `◐ SYNCING` | action |
| Failed | `● FAILED` | error |

The normal online state no longer displays `ONLINE`. Process states retain text
because they explain why catalog content may not yet be available. Failure
retains text because it requires attention.

## Catalog Rows

Database, schema, table, view, and other catalog labels no longer append
`native_kind`. Their existing icons already communicate the structural type.

Examples:

```text
▾ ◆ moss_biz
  ▾ ◇ tools
    ▾ Tables  79
      ▦ user_account  User accounts
```

Structural metadata remains visible where it adds information. Column
nullability/defaults, index columns, and constraint definitions remain metadata;
the repeated `database`, `schema`, or `table` suffix is removed.

## Counts

`CatalogCount` must use user-facing formatting instead of Rust `Debug` output:

| Value | Display |
| --- | --- |
| `Exact(79)` | `79` |
| `AtLeast(79)` | `79+` |
| `Unknown` | omitted |

This preserves exactness semantics without exposing enum implementation details.

## Comments and Metadata

Object label, structural metadata, and comment become separate semantic fields
and separate Ratatui spans:

- Label uses the catalog-kind primary color.
- Structural metadata uses normal secondary text.
- Comment uses `theme.muted`.
- Selected rows keep the selection background and primary-label emphasis while
  preserving status/comment hierarchy where contrast permits.
- Comment truncates before the label.

No punctuation wrapper is added around comments. Two spaces separate adjacent
fields.

## Projection Boundary

`VisibleCatalogNode` remains a UI-neutral projection type. It will expose the
semantic values needed by the renderer rather than a preformatted combined
string. Expected concepts include:

- primary label
- structural metadata
- comment
- catalog kind
- profile database kind
- profile provenance
- connection status
- endpoint
- expandability and tree identity

The exact Rust representation may use optional fields or a typed row-presentation
enum, whichever produces the smallest clear implementation. It must not contain
Ratatui `Span`, `Style`, or `Color` values.

Catalog entries, adapter responses, normalized Explorer identity, lazy loading,
selection, and expansion state remain unchanged.

## Safety and Performance

All user- or server-provided strings remain sanitized before rendering. Span
construction must operate only on the visible viewport rows and must not add
catalog-tree scans. The existing 10,000-object projection performance contract
must continue to pass.

## Verification

- Keymap tests distinguish `o` from `Enter` in Explorer focus.
- Reducer tests prove `o` cannot open relation previews.
- `Enter` opens data preview from relations and relation descendants.
- Contextual help and footer document the split behavior.
- Saved roots omit `SAVED`; session roots retain muted `SESSION`.
- PostgreSQL, MySQL, and SQLite roots use their configured Nerd Font icon.
- All connection states use the approved shape, text, and color hierarchy.
- Catalog labels omit redundant native-kind suffixes.
- Counts render as number, number-plus, or nothing.
- Comments render separately in the muted style.
- Narrow layouts preserve primary identity and safely truncate secondary text.
- Terminal-control sanitization and Explorer performance tests continue to pass.
