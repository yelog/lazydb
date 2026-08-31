# Visible Objects State And Loading Design

**Status:** Approved on 2026-08-31

## Goal

Make the Visible Objects picker easier to scan and make catalog discovery visibly asynchronous. Replace `[ ]`, `[-]`, and `[x]` with mode-aware semantic icons, preserve saved rows while loading, and prevent edits from racing with refresh.

## Decisions

- Nerd Font uses the MDI family: `󰄱` unchecked, `󰄲` checked, `󰡖` partial.
- Unicode uses `☐`, `☑`, and `▣`; ASCII retains `[ ]`, `[x]`, and `[-]`.
- Icons are mapped by `IconSet`, not embedded in the profile renderer.
- Unchecked, checked, and partial icons use `theme.muted`, `theme.accent`, and `theme.warning` respectively.
- Loading uses the shared `ActivityIndicator`, retains known rows, and disables toggle/refresh while keeping navigation and back actions available.

## Loading Flow

The existing `scope_discovery_request` remains the loading source of truth. Its request id is observed by the UI animation state as a `ProfileScope` load identity. The picker renders `Loading visible objects` with discovery detail above preserved rows, or `Waiting for catalog discovery...` when no rows exist. Completion restores normal hints and hit regions; failure preserves saved selections and shows the existing sanitized warning.

## Testing

Cover icon mappings and safe fallback output for all three modes, tri-state rendering and colors, loading/preserved rows, empty loading, hit-region suppression, model mutation protection, keymap suppression, and success/failure recovery. Run focused tests, `cargo test`, formatting, and Clippy with warnings denied.
