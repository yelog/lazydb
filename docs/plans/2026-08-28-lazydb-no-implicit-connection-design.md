# LazyDB No Implicit Connection Design

## Goal

Prevent a normal LazyDB launch, including workspace restoration, from opening a database connection automatically. Keep automatic connection and expansion for an explicitly supplied `--profile` or `--url`, and keep manual connection on `o` or `Enter`.

## Current Cause

`load_startup_profiles` resolves `selected` from explicit CLI arguments, then falls back to the first persisted profile. `run_tui` passes this value to `apply_startup_action_with_runtime`, which dispatches `RequestProfileConnect`. Therefore a normal launch turns the first profile into an implicit connection request.

## Design

Use `StartupProfiles.selected` only as the explicit startup connection target:

- `--url` selects the newly imported ad-hoc profile and keeps automatic connection.
- `--profile <name>` selects the named persisted profile and keeps automatic connection.
- No connection argument leaves `selected` as `None`.
- Workspace restoration receives the same value, so it restores layout/state without connecting.
- The explorer continues to initialize and select/display profiles normally. Its default first row is presentation state, not a connection request.
- Existing `RequestProfileConnect` handling remains the single path for user-triggered `o`/`Enter` connections.

## Error Handling

Unknown explicit profile names continue to return the existing startup error. No new error or connection state is introduced.

## Testing

Add a regression assertion that a non-empty profile store with no CLI connection arguments produces `selected == None`. Preserve tests for explicit profile and URL selection, including startup password binding.
