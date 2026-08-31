# Visible Objects State And Loading Design

**Status:** Approved on 2026-08-31

## Goal

Make the Visible Objects picker easier to scan and make catalog discovery visibly
asynchronous. Replace the hard-coded checkbox text with semantic icons and colors,
keep saved selections visible while discovery runs, and prevent edits from racing
with a refresh.

This design extends the existing Visible Objects discovery and TUI icon systems. It
does not change catalog-scope persistence, discovery results, or database adapter
behavior.

## State Icons

`IconSet` owns the three selection-state mappings so the picker follows the active
`--icons` mode and does not embed private-use characters in its renderer.

| State | Nerd Font | Unicode | ASCII |
| --- | --- | --- | --- |
| Unchecked | `md::MD_CHECKBOX_BLANK_OUTLINE` (`󰄱`) | `☐` | `[ ]` |
| Checked | `md::MD_CHECKBOX_MARKED` (`󰄲`) | `☑` | `[x]` |
| Partial | `md::MD_CHECKBOX_INTERMEDIATE` (`󰡖`) | `▣` | `[-]` |

All Nerd Font glyphs come from the Material Design Icons family exposed by the
existing `nerd-font-symbols` dependency. This keeps their box dimensions, stroke,
and baseline more consistent than mixing the Seti unchecked glyph with MDI checked
and intermediate glyphs.

The renderer reserves a state-prefix column based on terminal cell width rather
than assuming every icon occupies one cell. Unicode mappings contain no private-use
characters, and ASCII mappings remain strictly ASCII.

## Color And Row Styling

The state icon and object label render as separate spans:

- Unchecked uses `theme.muted`.
- Checked uses `theme.accent`.
- Partial uses `theme.warning`.
- Available object names use `theme.text`.
- Unavailable object names retain `theme.warning` and the existing `(mirrored)`
  suffix.
- The focused row retains `theme.selection` as its background.

Color is supplemental. Each state has a distinct shape and textual fallback, so
the state remains understandable in monochrome terminals and for users with color
vision deficiencies. A checked state uses the existing accent instead of adding a
new success color to the theme.

## Loading Presentation

Opening the picker without a fresh matching snapshot, or pressing `r`, starts the
existing fingerprinted discovery request. While it is pending:

- Render the shared `ActivityIndicator` in a fixed first content row with the label
  `Loading visible objects` and detail `discovering databases and schemas`.
- Respect the configured motion mode and icon mode. Motion-off mode renders the
  shared static marker rather than introducing a second spinner implementation.
- Keep saved or previously discovered rows visible below the indicator.
- Dim row labels while retaining enough icon color to communicate their saved
  state.
- If there are no rows, show `Waiting for catalog discovery...` below the indicator
  instead of leaving the panel empty.
- Replace the normal hint with `Loading...   Enter back   Esc back`.

On success, remove the loading row and restore the normal rows and hint. On global
failure, keep saved selections and render the existing sanitized failure warning.
Partial-discovery warnings continue through the existing warning path.

The picker follows stale-while-revalidate behavior. It never clears known scope
rows merely to communicate progress, and it does not show fake percentages because
discovery exposes no reliable total progress.

## Loading Interaction

Selection changes and refresh are disabled while discovery is pending. Navigation
and leaving the picker remain available.

The restriction is enforced at multiple relevant boundaries:

- The renderer does not create scope-row mouse hit regions while loading.
- Keyboard and mouse action handling ignore toggle and refresh requests while
  loading.
- `ProfileManagerState::toggle_scope_row` refuses changes while loading, protecting
  future callers that bypass the current input path.
- `Enter` and `Esc` continue to return to the profile form.

This avoids applying a toggle to stale candidates immediately before a discovery
response updates the rendered tree. Existing request-id and fingerprint checks
continue to reject stale asynchronous responses.

## Architecture And Data Flow

No new domain operation is introduced. The existing
`scope_discovery_request: Option<(u64, DiscoveryFingerprint)>` remains the source of
truth for loading state.

```text
open/refresh Visible Objects
  -> begin_scope_discovery
  -> scope_discovery_request = Some(...)
  -> render ActivityIndicator + preserved rows
  -> block toggle/refresh, allow navigation/back
  -> discovery success or failure
  -> scope_discovery_request = None
  -> render updated/preserved rows + warning when applicable
```

Presentation timing and spinner frames reuse the UI animation state and shared
loading widget. Loading state remains in `ProfileManagerState`; elapsed time and
motion behavior remain UI-only.

## Testing

- Extend `IconSet` unit tests with all three scope-selection states in Nerd Font,
  Unicode, and ASCII modes.
- Verify mappings are non-empty and control-character-free, Unicode mappings avoid
  private-use code points, and ASCII mappings are ASCII-only.
- Add picker render tests for unchecked, checked, and partial state symbols.
- Verify loading output includes the activity text and preserves known rows.
- Verify an empty loading picker shows the waiting message.
- Verify loading rows do not produce `ProfileScopeRow` hit regions and the loading
  hint replaces the normal toggle/refresh hint.
- Add model and reducer coverage proving loading blocks selection changes and
  duplicate refresh while back navigation remains available.
- Verify success restores normal interaction and failure preserves saved rows with
  the sanitized error.

Final verification runs `cargo fmt --check`, the focused icon/profile/UI/mouse
tests, `cargo test`, and `cargo clippy --all-targets --all-features -- -D warnings`.
