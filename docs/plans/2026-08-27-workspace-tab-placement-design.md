# Workspace Tab Placement Design

## Scope

Move the heterogeneous SQL console and relation-preview tab bar into the main
content column, preserve relation-tab reuse, and add a shortcut that returns to
the first available SQL console. Do not split SQL and relation tabs into
separate state models or change workspace persistence.

## Current Behavior

`WorkspaceTab` already models SQL consoles and relation previews as peers, and
`App::active_tab` selects which main-content page is active. Relation previews
are already deduplicated by `RelationKey`, composed of profile UUID and catalog
object ID.

The visual hierarchy is incorrect because `AppLayout` allocates the tab row
before splitting the body into Explorer and main columns. The full-width tab row
therefore appears to control both columns even though it only selects the main
content.

## Layout

- Keep the application header and footer full-width.
- Split the area between them horizontally into Explorer and main columns.
- Let Explorer occupy the complete body height.
- Split the main column vertically into a two-row workspace tab bar and the
  active content page.
- For a SQL tab, split the active content page into SQL Editor, result-view tabs,
  and Results using the existing proportions.
- For a relation tab, give the complete active content page to the relation
  renderer. Its Data and Structure controls remain page-local secondary
  navigation.
- Remove the `WORKSPACE` prefix from the tab row. Keep tab sequence numbers,
  titles, active styling, and mouse hit regions.

In narrow focus mode, Explorer focus does not render the workspace tab bar
because the main content is hidden. Editor and Results focus render the tab bar
above the visible main-content page. This preserves the meaning that the tabs
select main content rather than the complete application.

## Relation Tab Reuse

Keep the existing `RelationKey { profile_id, object_id }` lookup as the sole
relation-tab identity mechanism.

Opening a relation or one of its supported descendants resolves the owning
relation. If a matching relation tab exists, activate it and select the requested
Data or Structure view. Otherwise, create a new relation tab and load it. The
profile UUID remains part of the key, so equally named objects from different
profiles do not collide.

No additional tab registry or cache is needed. Add focused reducer tests to make
the existing behavior explicit and prevent regressions.

## Goto SQL Console

Add a semantic `GotoSqlConsole` action mapped to `Space s`.

The reducer finds the first `WorkspaceTab::Sql` in tab order, activates it, and
sets focus to `Focus::Editor`. If the original startup console has been closed,
the first remaining SQL console is the target. The existing invariant that at
least one SQL console remains open means the action normally always has a
target.

The shortcut is available from Editor Normal mode, Explorer, Results, and
relation previews. It must not intercept Editor Insert mode, relation query text
input, or active overlays. Triggering it while the first SQL console is already
active still moves focus to Editor.

Document the shortcut in the keybinding reference and contextual help. Relation
page footer guidance may include `Space s SQL console`; the regular SQL footer
does not need another persistent hint.

## State and Data Flow

Input maps `Space s` to `Action::GotoSqlConsole`. `App::update` remains the only
state mutation boundary and changes only `active_tab` and `focus`. Rendering
projects the updated layout and records tab hit regions at their new main-column
coordinates. Mouse input continues to emit `Action::ActivateTab(index)`.

No runtime command, database request, persistence schema, or editor session
change is introduced.

## Error Handling

If no SQL tab is found, `GotoSqlConsole` is a no-op rather than panicking. This
defensively protects the reducer even though normal close behavior preserves at
least one SQL console.

Existing relation loading, cancellation, stale-response rejection, and retained
snapshot behavior remain unchanged.

## Testing

- Verify standard and wide layouts place the tab row at the Explorer right
  boundary and let Explorer use the complete body height.
- Verify relation pages use the main content below the relocated tab row.
- Verify narrow Explorer focus hides tabs while narrow Editor and Results focus
  place tabs above visible main content.
- Verify tab mouse hit regions use the relocated coordinates.
- Verify opening the same relation or one of its descendants reuses one relation
  tab and selects the requested view.
- Verify same-named relations from different profiles do not collide.
- Verify `Space s` activates the first SQL tab and focuses Editor from a relation
  tab and from SQL Results.
- Verify closing the original console causes the shortcut to select the first
  remaining SQL console.
- Verify Editor Insert mode, relation query input, and overlays do not consume
  `Space s` as the global shortcut.
- Run focused layout, UI, input, and reducer tests, followed by formatting,
  all-target tests, and strict Clippy checks.
