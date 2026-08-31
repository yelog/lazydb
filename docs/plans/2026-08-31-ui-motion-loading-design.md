# UI Motion And Loading Design

## Scope

Improve LazyDB's terminal interaction feedback without changing its keyboard-first
workflow or replacing the existing Ratatui widgets. The first release includes:

- `full`, `reduced`, and `off` motion modes exposed through `--motion`.
- A shared activity indicator with Unicode and ASCII frames.
- A delayed table skeleton for SQL queries that have no result to preserve.
- Animated relation preview and DDL refresh status.
- Dimmed modal backgrounds and short TachyonFX entrance effects.
- A short result-ready accent sweep after successful SQL and relation loads.
- Demand-driven redraws so idle sessions do not render continuously.

The change does not add popup exit animations, fake percentage progress, animated
completion menus, persisted motion preferences, or a general-purpose animation
framework outside the UI layer.

## Current Architecture

LazyDB renders directly through Ratatui 0.30.2 with Crossterm 0.29. The runtime
already owns a 33 ms Tokio ticker in `src/runtime.rs`, but a tick requests a redraw
only while a SQL console has `QueryStatus::Running` or a clipboard notice expires.
Relation loads and future finite UI effects therefore do not currently animate.

`UiState` persists across terminal draws and already stores render-derived state
such as hit regions and viewports. It is the correct owner for transient animation
state. Business state remains in `App`, `ConsoleTab`, `Overlay`, and
`RelationLoad`; none of those types should acquire frame counters, effect timers,
or terminal geometry.

SQL result rendering currently shows either the latest result or a static empty
message. Relation rendering already follows a stale-while-revalidate model by
keeping `RelationLoad::Loading::previous`, but its `Refreshing` status is static.
Overlays are rendered after the workspace using `Clear` and final popup geometry,
so they appear immediately without a depth transition.

## Architecture

Add a UI-only animation module under `src/ui/animation.rs`. `UiState` owns one
`AnimationState`, and the runtime advances it with a timestamp before each draw.
Rendering reads an immutable animation snapshot for deterministic frame selection
and applies post-render TachyonFX effects through the same state.

```text
App business state
       |
       v
UiState::animations <--- runtime 33 ms ticker / monotonic Instant
       |
       +--- ActivityIndicator and TableSkeleton
       |
       +--- overlay identity and result identity transitions
       |
       +--- TachyonFX EffectManager
       |
       v
Ratatui Frame buffer
```

Animation progress is elapsed-time based, not redraw-count based. Skipped ticks do
not slow finite effects, and tests can inject a fixed elapsed duration. The module
exposes a single `needs_redraw()` decision so the runtime does not duplicate
knowledge about active effects.

## Motion Modes

Define `MotionMode` beside the other CLI value enums:

- `Full`: dynamic spinner and skeleton, popup entrance effects, animated dimming,
  and result-ready sweeps.
- `Reduced`: low-frequency spinner, static skeleton, immediate background dimming,
  and no finite movement or sweep effects.
- `Off`: static loading marker and text, static skeleton, immediate background
  dimming, and no finite effects.

The CLI defaults to `full`. The mode is session-only and is passed from `Cli` to
`UiState`; it is not stored in connection profiles or workspace persistence.
Reduced and off modes still display textual status, cancellation hints, failures,
and stale-result labels. Motion is never the only status signal.

## Animation Identity And Lifecycle

The UI observes stable business identities rather than receiving new animation
actions:

- SQL execution identity: active console UUID plus console generation while
  `QueryStatus::Running`.
- Derived query identity: console UUID plus derived generation while
  `DerivedResultState::running`.
- Relation identity: the existing `RelationRequest` in `RelationLoad::Loading`.
- Overlay identity: a small presentation enum containing only the `Overlay`
  variant and stable object ID where required. Selection or input changes inside
  the same popup do not restart its entrance effect.
- Result identity: console UUID plus successful generation, or relation request
  identity after it changes from Loading to Ready.

`AnimationState::observe(app, now, viewport)` compares the current identities with
the previous frame. It starts, retains, cancels, or completes UI effects without
mutating `App`. Resize cancels effects tied to stale rectangles and allows the
current overlay to render immediately in its new geometry.

The SQL loading start time is kept in `AnimationState` under the execution
identity. This avoids changing persisted or domain state. A process that first
observes an already-running query starts elapsed display at zero, which is an
acceptable session-local presentation trade-off.

## Activity Indicator

Implement an internal widget rather than adding a spinner dependency. It accepts
the motion mode, icon mode, elapsed duration, label, optional detail, and optional
cancel hint.

Full and reduced Unicode modes use Braille frames:

```text
⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏
```

ASCII icon mode uses:

```text
| / - \
```

Full mode changes at roughly 10 frames per second, reduced mode at roughly five,
and off mode renders a stable `*`. Labels use concrete language such as
`Executing query`, `Refreshing relation data`, or `Loading DDL`. After one second,
the widget appends elapsed time. It never reports a percentage unless the
underlying operation has real progress data, which is outside this scope.

## Table Skeleton

When a SQL or derived query is still running, has no previous result, and has
remained running for at least 150 ms, render a table-shaped placeholder inside the
existing result panel. Before 150 ms, keep the panel structure and render only a
quiet status line to avoid a visible flash for fast queries.

The skeleton derives all geometry from the destination `Rect`:

- One activity/status row.
- Three placeholder columns when the width permits, otherwise one or two.
- Four to six body rows depending on height.
- Stable widths for the lifetime of a frame.
- `░` and `▒` bands in full mode, with the band position derived from elapsed
  time.
- A static muted pattern in reduced and off modes.

The skeleton does not write `UiState::grid_viewport`, expose cell hit regions, or
pretend that placeholder cells are selectable. Existing results remain visible
during refresh; the skeleton is only for the no-result case.

## SQL And Relation Loading

SQL rendering distinguishes four states:

- Idle with no result: keep `Run a query to populate the data viewport`.
- Running with no result: delayed activity indicator and skeleton.
- Running with a previous result: keep the table readable and render a raised
  status strip saying `Executing query · showing previous result` with elapsed
  time and the existing cancel shortcut.
- Finished: render the real result and, in full mode, start one short accent sweep
  when a new successful result identity is observed.

Derived query loading follows the same presentation using
`DerivedResultState::running` and its generation.

Relation Data and DDL continue using the existing `previous` snapshot. Their
static `Refreshing` text is replaced by the shared activity indicator and a clear
`showing previous snapshot` detail when previous content exists. A first relation
load with no snapshot shows the indicator and a suitable skeleton or empty DDL
surface. Failure and cancellation remain static and preserve retry/cancel hit
targets.

## Overlay Motion

Render the complete workspace first, then dim the background before drawing the
overlay. Dimming changes foreground and background colors only; it does not erase
content, alter layout, or intercept input. Reduced and off modes apply the final
dim immediately. Full mode interpolates to the dimmed colors over approximately
120 ms.

Each overlay renderer continues to own its final popup `Rect`. After it renders,
the animation layer applies a 140-180 ms TachyonFX entrance effect limited to that
rectangle. Use restrained effects only:

- `fade_from_fg` for content.
- `sweep_in` or `expand` for the border/content reveal.
- `parallel` with `QuadOut` or `CubicOut` timing.

Do not use bounce, elastic, glitch, explode, full-screen dissolve, or perpetual
decorative effects. Closing an overlay remains immediate. A new overlay replaces
the old effect immediately. Completion popups are excluded because typing would
restart effects too frequently.

The overlay renderer must report its popup rectangle to `UiState` so the effect
area is exact. Large overlays such as Help, Record View, and Profile Manager use
their actual panel bounds rather than the full terminal.

## Result-Ready Feedback

When a successful SQL or relation result identity first appears, full mode applies
a 200-300 ms foreground/accent sweep to the result panel, preferably limited to
the title, header, and status cells. Reduced and off modes display the result
immediately. The effect runs once per identity and never repeats merely because
the terminal redraws or resizes.

Failure and cancellation do not run the success sweep. They retain existing text
and colors, with no shake or flashing effect.

## Runtime Redraw Policy

Keep the existing 33 ms ticker and `MissedTickBehavior::Skip`. On each tick, ask
whether any of these conditions needs a redraw:

- A clipboard notice expires.
- A SQL or derived query is running and its current motion mode requires a visual
  frame or elapsed-time update.
- A relation preview or DDL request is loading.
- A finite TachyonFX effect is active.
- A delayed loading threshold is about to become visible.

`MotionMode::Off` should not redraw at 30 FPS. It needs a redraw only at discrete
presentation boundaries such as the 150 ms loading threshold and whole-second
elapsed updates. `AnimationState` returns the next deadline or a boolean suitable
for the current ticker; the initial implementation may keep the 33 ms interval as
long as it declines the actual terminal draw while no visual output changed.

Input events and runtime actions still request immediate redraws. Animation must
never delay keyboard handling, cancellation, resize, or application shutdown.

## Error Handling And Compatibility

- Too-small layout bypasses effects and renders the existing fallback.
- A zero-sized popup or result rectangle does not create an effect.
- Resize invalidates rectangle-bound effects before rendering the new layout.
- ASCII icon mode never emits Braille or private-use characters.
- Low-color terminals retain readable static foreground/background contrast.
- TachyonFX is presentation-only; inability to start an effect falls back to the
  final rendered frame without affecting business operations.
- Sanitization rules remain unchanged. Animation labels use fixed application
  strings; database-derived text still goes through existing sanitizers.

## Testing

Unit tests cover:

- CLI parsing and defaults for all motion modes.
- Spinner frames, ASCII fallback, frame rate, and stable off mode.
- Loading delay and elapsed-time formatting.
- Skeleton geometry in narrow, short, and normal areas.
- Identity observation without restarting effects on ordinary redraws.
- Overlay replacement, closure, and resize invalidation.
- `needs_redraw()` behavior for full, reduced, off, idle, and finite effects.

Buffer render tests use a fixed animation timestamp and verify:

- SQL first-load skeleton.
- SQL stale-result status.
- Relation Data and DDL loading with and without previous snapshots.
- Static reduced/off rendering.
- Popup background dimming and effect area reporting.
- Result-ready effects do not change table content or hit regions.

Run focused UI and CLI tests first, then `cargo fmt --check`, `cargo test`, and
`cargo clippy --all-targets --all-features -- -D warnings`.

## Rollout Boundaries

The first release is complete when all loading operations provide clear feedback,
full mode adds restrained popup/result motion, reduced/off modes are deterministic,
and an idle session performs no continuous terminal draws. Future work may add a
persisted config-file preference, Explorer-node activity indicators, popup exit
effects, and long-content scroll widgets, but none are prerequisites here.
