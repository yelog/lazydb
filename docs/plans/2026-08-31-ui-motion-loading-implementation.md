# UI Motion And Loading Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add accessible, demand-driven loading feedback and restrained popup/result animations to LazyDB's Ratatui interface.

**Architecture:** Keep business state unchanged and add a persistent UI-only animation runtime inside `UiState`. Render custom activity/skeleton widgets from elapsed time, use TachyonFX only for finite buffer effects, and let the existing 33 ms runtime ticker draw only when the animation runtime reports a visible change.

**Tech Stack:** Rust 2024, Ratatui 0.30.2, Crossterm 0.29, Tokio 1.47, TachyonFX 0.25.1, Clap 4.5

---

## Preconditions And Invariants

- Work in `/Users/yelog/workspace/tui/lazydb` unless the user supplies a separate worktree.
- Follow `docs/plans/2026-08-31-ui-motion-loading-design.md`.
- The worktree is already dirty. Preserve all unrelated user changes, especially current edits in `src/app.rs`, `src/runtime.rs`, `src/ui/mod.rs`, and `src/ui/relation.rs`.
- Do not add animation fields to persisted workspace/profile models.
- Do not change query, relation, transaction, cancellation, keyboard, or mouse semantics.
- Do not animate completion popups or add popup exit states in this release.
- Do not render fake percentages or make placeholder cells selectable.
- Keep ASCII icon mode strictly ASCII.
- Do not commit unless the user explicitly requests it. Any workflow that normally commits after each task must skip commit steps.

### Task 1: Add The Motion Mode CLI Contract

**Files:**
- Modify: `src/cli.rs:3-70,228-end`
- Modify: `src/runtime.rs:2529-2536`
- Test: `src/cli.rs`

**Step 1: Write failing CLI parsing tests**

Add tests beside the existing `Cli::try_parse_from` coverage:

```rust
#[test]
fn motion_defaults_to_full() {
    let cli = Cli::try_parse_from(["lazydb"]).unwrap();
    assert_eq!(cli.motion, MotionMode::Full);
}

#[test]
fn parses_all_motion_modes() {
    for (value, expected) in [
        ("full", MotionMode::Full),
        ("reduced", MotionMode::Reduced),
        ("off", MotionMode::Off),
    ] {
        let cli = Cli::try_parse_from(["lazydb", "--motion", value]).unwrap();
        assert_eq!(cli.motion, expected);
    }
}
```

Also assert that an unknown value fails parsing.

**Step 2: Run the focused test and verify failure**

Run: `cargo test cli::tests --lib`

Expected: FAIL because `MotionMode` and `Cli::motion` do not exist.

**Step 3: Define the CLI enum and field**

Add near `MouseMode` and `ColorMode`:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum MotionMode {
    #[default]
    Full,
    Reduced,
    Off,
}
```

Add to `Cli`:

```rust
#[arg(long, global = true, value_enum, default_value_t = MotionMode::Full)]
pub motion: MotionMode,
```

Do not add this value to persisted profiles or workspace snapshots.

**Step 4: Pass the selected mode into UI state construction**

Replace the unconditional `UiState::default()` in `run_tui` with a constructor
that accepts `cli.motion`. The constructor is introduced in Task 2; until then,
use a temporary compile error rather than duplicating state.

**Step 5: Run focused tests**

Run: `cargo test cli::tests --lib`

Expected: PASS.

### Task 2: Build A Deterministic UI Animation Clock

**Files:**
- Create: `src/ui/animation.rs`
- Modify: `src/ui/mod.rs:1-43,106-199`
- Modify: `src/cli.rs` only if a visibility adjustment is required
- Test: `src/ui/animation.rs`

**Step 1: Write failing tests for mode cadence and loading delay**

Create module tests for pure calculations. Cover these requirements:

```rust
#[test]
fn full_motion_advances_spinner_every_hundred_milliseconds() {
    assert_eq!(spinner_frame(MotionMode::Full, Duration::ZERO, 10), 0);
    assert_eq!(spinner_frame(MotionMode::Full, Duration::from_millis(99), 10), 0);
    assert_eq!(spinner_frame(MotionMode::Full, Duration::from_millis(100), 10), 1);
}

#[test]
fn reduced_motion_uses_a_lower_cadence_and_off_is_stable() {
    assert_eq!(spinner_frame(MotionMode::Reduced, Duration::from_millis(199), 10), 0);
    assert_eq!(spinner_frame(MotionMode::Reduced, Duration::from_millis(200), 10), 1);
    assert_eq!(spinner_frame(MotionMode::Off, Duration::from_secs(10), 10), 0);
}

#[test]
fn skeleton_is_hidden_before_the_delay() {
    assert!(!show_skeleton(Duration::from_millis(149)));
    assert!(show_skeleton(Duration::from_millis(150)));
}
```

**Step 2: Run the focused tests and verify failure**

Run: `cargo test ui::animation::tests --lib`

Expected: FAIL because the animation module and calculations do not exist.

**Step 3: Implement pure timing helpers**

Add constants and helpers equivalent to:

```rust
pub(crate) const LOADING_DELAY: Duration = Duration::from_millis(150);

pub(crate) fn spinner_frame(mode: MotionMode, elapsed: Duration, frames: usize) -> usize {
    if frames == 0 || mode == MotionMode::Off {
        return 0;
    }
    let frame_ms = match mode {
        MotionMode::Full => 100,
        MotionMode::Reduced => 200,
        MotionMode::Off => unreachable!(),
    };
    (elapsed.as_millis() / frame_ms % frames as u128) as usize
}

pub(crate) fn show_skeleton(elapsed: Duration) -> bool {
    elapsed >= LOADING_DELAY
}
```

Use saturating duration calculations anywhere an `Instant` can be observed out of
order in a test.

**Step 4: Define persistent animation state**

Start with a minimal state that can grow in later tasks:

```rust
pub(crate) struct AnimationState {
    mode: MotionMode,
    now: Instant,
    last_frame_at: Instant,
    active_loads: HashMap<LoadIdentity, Instant>,
}
```

Define presentation-only identities:

```rust
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum LoadIdentity {
    Query { tab_id: Uuid, generation: u64 },
    Derived { tab_id: Uuid, generation: u64 },
    Relation(RelationRequest),
}
```

Expose narrow methods rather than public fields:

- `new(mode, now)`
- `mode()`
- `set_now(now)` for runtime
- `track_load(identity)`
- `finish_load(identity)`
- `elapsed(identity)`
- `has_active_loads()`

**Step 5: Add deterministic test construction**

Extend `UiState` with `animations: AnimationState` and constructors:

```rust
pub fn new() -> Self
pub fn with_motion(mode: MotionMode) -> Self
#[cfg(test)]
pub(crate) fn with_motion_at(mode: MotionMode, now: Instant) -> Self
```

Keep `UiState::new()` and `Default` behavior equivalent to `MotionMode::Full` so
existing tests continue to compile unchanged.

**Step 6: Test load tracking does not restart**

Add a test that tracks one identity at `t0`, observes it again at `t0 + 500ms`,
and verifies elapsed remains 500 ms. Finish it and verify it is removed.

**Step 7: Run focused tests**

Run: `cargo test ui::animation::tests --lib`

Expected: PASS.

### Task 3: Implement Shared Activity And Skeleton Rendering

**Files:**
- Create: `src/ui/loading.rs`
- Modify: `src/ui/mod.rs:1-43`
- Modify: `src/ui/icons.rs:18-185`
- Test: `src/ui/loading.rs`
- Test: `src/ui/icons.rs`

**Step 1: Write failing icon fallback tests**

Add an `IconSet` method that exposes whether ASCII-safe animation symbols are
required, or preferably add a narrow method returning the spinner frame set. Test:

```rust
#[test]
fn ascii_motion_frames_are_ascii() {
    let icons = IconSet::new(IconMode::Ascii);
    assert!(icons.activity_frames().iter().all(|frame| frame.is_ascii()));
}

#[test]
fn unicode_motion_frames_do_not_use_private_use_characters() {
    for mode in [IconMode::NerdFont, IconMode::Unicode] {
        let icons = IconSet::new(mode);
        assert!(icons.activity_frames().iter().all(|frame| {
            frame.chars().all(|character| !is_private_use(character))
        }));
    }
}
```

**Step 2: Run icon tests and verify failure**

Run: `cargo test ui::icons::tests --lib`

Expected: FAIL because `activity_frames` does not exist.

**Step 3: Add safe frame sets**

Use fixed static slices:

```rust
const BRAILLE_ACTIVITY: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_ACTIVITY: &[&str] = &["|", "/", "-", "\\"];
```

Nerd Font and Unicode modes use Braille because the symbols are standard Unicode,
not private-use glyphs. Off mode is handled by the loading renderer and uses `*`.

**Step 4: Write failing activity-line tests**

Test a pure builder rather than inspecting terminal timing:

```rust
#[test]
fn activity_line_includes_elapsed_after_one_second() {
    let line = activity_text(
        "Executing query",
        Some("showing previous result"),
        true,
        Duration::from_millis(1_240),
    );
    assert_eq!(line, "Executing query · showing previous result · 1.2s · Ctrl-C cancel");
}

#[test]
fn activity_line_omits_subsecond_elapsed() {
    let line = activity_text("Loading relation data", None, false, Duration::from_millis(900));
    assert_eq!(line, "Loading relation data");
}
```

Use one decimal below ten seconds and whole seconds at ten seconds or above. Do
not add a separate duration-formatting abstraction outside `loading.rs`.

**Step 5: Write failing skeleton geometry tests**

Render `TableSkeleton` into `Buffer::empty(Rect)` with fixed elapsed values and
assert:

- No write occurs outside the supplied `Rect`.
- A normal 60x10 area has three column regions and at least four body rows.
- A 20x4 area degrades to one column without panicking.
- Full mode changes the `░`/`▒` band between fixed timestamps.
- Reduced and off output is identical across timestamps.
- ASCII icon mode contains no non-ASCII spinner character; skeleton shading may
  use ASCII `.`/`=` in ASCII mode to keep the entire widget ASCII.

**Step 6: Run loading tests and verify failure**

Run: `cargo test ui::loading::tests --lib`

Expected: FAIL because `ActivityIndicator` and `TableSkeleton` do not exist.

**Step 7: Implement the widgets**

Implement small `Widget` values that receive all state explicitly:

```rust
pub(crate) struct ActivityIndicator<'a> {
    pub mode: MotionMode,
    pub icons: IconSet,
    pub elapsed: Duration,
    pub label: &'a str,
    pub detail: Option<&'a str>,
    pub cancellable: bool,
    pub style: Style,
}

pub(crate) struct TableSkeleton {
    pub mode: MotionMode,
    pub icons: IconSet,
    pub elapsed: Duration,
    pub theme: Theme,
    pub block: Block<'static>,
}
```

If owning `Block<'static>` makes call sites awkward, pass title/focus and build the
block inside `loading.rs`; do not add clones throughout callers merely to satisfy
the draft signature.

Reserve the first inner row for `ActivityIndicator`. Build placeholder widths from
the current inner width and clamp every write through Ratatui `Rect` APIs. Do not
push hit regions or update a grid viewport.

**Step 8: Run focused tests**

Run:

```bash
cargo test ui::icons::tests --lib
cargo test ui::loading::tests --lib
```

Expected: PASS.

### Task 4: Observe Business Loading State Without Mutating It

**Files:**
- Modify: `src/ui/animation.rs`
- Modify: `src/ui/mod.rs:210-302`
- Modify: `src/model/relation.rs:105-123` only if a non-mutating identity helper is needed
- Test: `src/ui/animation.rs`

**Step 1: Write failing observation transition tests**

Use small fixtures or a presentation snapshot type so the tests do not need a full
database profile. Cover:

- Idle to running creates a query load identity once.
- Re-observing running does not reset its start time.
- Query generation change replaces the old identity.
- Running to idle finishes the identity and emits one result-ready transition only
  if a new result exists.
- Relation Loading uses its `RelationRequest` identity.
- Relation Loading to Ready emits one result-ready transition.
- Loading to Failed/Cancelled does not emit success.
- A derived query uses `DerivedResultState::generation` independently from the
  source query.

**Step 2: Run tests and verify failure**

Run: `cargo test ui::animation::tests --lib`

Expected: FAIL because observation and transitions do not exist.

**Step 3: Add a narrow animation observation snapshot**

Avoid storing or cloning all of `App`. Derive a per-frame snapshot containing only
active identities and successful result identities:

```rust
pub(crate) struct AnimationObservation {
    pub active_loads: HashSet<LoadIdentity>,
    pub result: Option<ResultIdentity>,
    pub overlay: Option<OverlayIdentity>,
}
```

Construct it in `ui::render_with_state_using_icons` or a private helper from the
current active tab and overlay. Keep SQL/relation matching in the UI layer.

**Step 4: Implement state reconciliation**

`AnimationState::observe` must:

- Insert newly active loads with the current UI timestamp.
- Retain existing start times.
- Remove inactive loads.
- Queue one result-ready event when a new successful result identity replaces the
  prior identity.
- Never treat an ordinary redraw as a new result.

Cap retained one-shot identities to current/previous state rather than growing an
unbounded history set.

**Step 5: Call observation before rendering**

At the top of `render_with_state_using_icons`, after Too Small can be determined
but before child rendering, reconcile the current observation. Ensure existing
`UiState` viewport resets remain unchanged.

**Step 6: Run focused and App tests**

Run:

```bash
cargo test ui::animation::tests --lib
cargo test app::tests --lib
```

Expected: PASS.

### Task 5: Render SQL And Derived Query Loading States

**Files:**
- Modify: `src/ui/mod.rs:1490-1620`
- Modify: `tests/ui_render.rs:243-288, relevant result tests`
- Test: `tests/ui_render.rs`

**Step 1: Add fixed-time test render helpers**

Extend test helpers without changing production APIs unnecessarily:

```rust
fn render_at(
    app: &App,
    width: u16,
    height: u16,
    motion: MotionMode,
    now: Instant,
) -> (String, UiState)
```

Create `UiState::with_motion_at`, render once to register a load at `t0`, advance
to `t0 + duration`, and render again when testing elapsed animations.

**Step 2: Write failing first-load tests**

Create an active SQL tab with `QueryStatus::Running`, no derived/source outcome,
and ASCII icons. Assert:

- At 149 ms, output contains `Executing query` but no skeleton body marker.
- At 150 ms, output contains the result panel, activity line, ASCII spinner, and
  placeholder rows.
- `UiState::grid_viewport` remains `None` and no result-cell hit region exists.
- Off mode output is stable between 150 ms and 500 ms except when elapsed text
  crosses a documented whole-second boundary.

**Step 3: Write failing stale-result tests**

Set `query_status = Running` while leaving an existing outcome. Assert the actual
cell value remains rendered and the status contains:

```text
Executing query · showing previous result
```

The table remains navigable and `grid_viewport` is still populated.

**Step 4: Write failing derived-query tests**

Set `DerivedResultState::running = true`. Cover no-derived-result and previous
derived-result states, and ensure derived loading takes precedence over the idle
source query presentation.

**Step 5: Run focused tests and verify failure**

Run exact test names with:

```bash
cargo test --test ui_render sql_query_loading
cargo test --test ui_render sql_query_refresh
cargo test --test ui_render derived_query_loading
```

Expected: FAIL because `render_data` has no loading branches.

**Step 6: Refactor `render_data` into explicit presentation states**

Derive a private enum inside `src/ui/mod.rs` or `loading.rs`:

```rust
enum ResultPresentation<'a> {
    Empty,
    Loading { identity: LoadIdentity, previous: Option<&'a ResultSet> },
    Ready(&'a ResultSet),
}
```

Prefer derived outcome/running state when active, preserving the existing derived
result precedence. For Loading without previous data, call `TableSkeleton` after
the 150 ms threshold. Before threshold, render the existing result block and a
quiet `ActivityIndicator` only. For Loading with previous data, allocate a two-row
status strip above the existing grid and pass the remaining area unchanged to
`render_result_table`.

Do not clone result sets or alter `ConsoleTab::outcome`.

**Step 7: Preserve completion popup ordering**

Render data-query completion after the table/loading surface exactly as today so
it remains the top-most local popup. Do not animate it.

**Step 8: Run focused render tests**

Run:

```bash
cargo test --test ui_render sql_query_loading
cargo test --test ui_render sql_query_refresh
cargo test --test ui_render derived_query_loading
```

Expected: PASS.

### Task 6: Render Relation Data And DDL Loading States

**Files:**
- Modify: `src/ui/relation.rs:99-329`
- Modify: `tests/ui_render.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing relation Data tests**

Create relation fixtures for:

- `RelationLoad::Loading { previous: None }`.
- `RelationLoad::Loading { previous: Some(...) }`.
- Reduced and off modes.

Assert first load says `Loading relation data`, a previous load preserves table
values and says `Refreshing relation data · showing previous snapshot`, and the
cancel hit region remains available.

**Step 2: Write failing relation DDL tests**

Repeat for DDL with and without a previous snapshot. A previous DDL body remains
visible; first load renders a stable empty surface and activity status. Error and
cancelled states remain static.

**Step 3: Run focused tests and verify failure**

Run:

```bash
cargo test --test ui_render relation_data_loading
cargo test --test ui_render relation_ddl_loading
```

Expected: FAIL because relation loading uses static `Refreshing` text.

**Step 4: Make `render_status` support activity presentation**

Do not overload booleans further. Replace `(message, retry, cancel)` tuples with a
small private enum:

```rust
enum RelationStatus<'a> {
    Loading { identity: LoadIdentity, label: &'static str, has_previous: bool },
    Failed(&'a str),
    Cancelled,
    Empty(&'static str),
}
```

Render Loading through `ActivityIndicator`; preserve the existing retry/cancel
labels and hit targets for non-loading states. Loading retains the current
`Ctrl-C cancel` target.

**Step 5: Use skeleton only for first Data load**

For relation Data with no previous result, render `TableSkeleton` after the shared
delay. For DDL first load, render the panel and activity strip without pretending
that SQL lines exist. Keep query-bar completion ordering intact.

**Step 6: Run focused tests**

Run:

```bash
cargo test --test ui_render relation_data_loading
cargo test --test ui_render relation_ddl_loading
```

Expected: PASS.

### Task 7: Add Demand-Driven Runtime Redraws

**Files:**
- Modify: `src/ui/animation.rs`
- Modify: `src/ui/mod.rs:106-199`
- Modify: `src/runtime.rs:2532-2612`
- Test: `src/ui/animation.rs`
- Test: `src/runtime.rs` or a new focused unit-test module in the same file

**Step 1: Write failing redraw-decision tests**

Test a pure method returning whether the current tick changes visible output:

- Idle state returns false.
- Full active spinner returns true only when its 100 ms frame boundary changes.
- Reduced spinner returns true only at 200 ms boundaries.
- Off loading returns true at the 150 ms skeleton threshold and elapsed-label
  boundaries, not every 33 ms.
- An active finite effect requests redraw while unfinished.
- Completing/removing the last load returns false after the final state redraw.

Represent this as `needs_redraw(previous_now, now)` or `next_deadline()`; prefer
the smallest API that keeps runtime free of motion rules.

**Step 2: Run tests and verify failure**

Run: `cargo test ui::animation::tests --lib`

Expected: FAIL because redraw scheduling does not exist.

**Step 3: Implement the animation scheduling decision**

Keep the 33 ms Tokio interval. On a ticker event:

```rust
let now = Instant::now();
let animation_redraw = ui_state.advance_animations(now);
redraw = app.expire_clipboard_notice(now) || animation_redraw;
```

`advance_animations` updates the animation clock once and reports whether the
rendered frame would differ. Do not inspect `QueryStatus` or `RelationLoad`
directly in `runtime.rs`; those facts were reconciled into `AnimationState` by the
previous draw.

Runtime actions that start/finish work already set `redraw = true`; the following
draw updates observed identities before the next ticker.

**Step 4: Ensure time advances before every event-driven draw**

Before rendering after keyboard, mouse, resize, paste, or runtime action, set the
UI animation timestamp to the same `Instant::now()` used for that frame. This
prevents finite effects from seeing stale time when redraws happen between ticks.

**Step 5: Test the runtime helper**

If extracting a private pure helper makes runtime tests straightforward, test it
in `src/runtime.rs`. Do not instantiate a real terminal merely to test a boolean.

**Step 6: Run focused tests**

Run:

```bash
cargo test ui::animation::tests --lib
cargo test runtime --lib
```

Expected: PASS, and idle state no longer depends on a hard-coded query-running
scan in the ticker branch.

### Task 8: Add TachyonFX And A Finite Effect Manager

**Files:**
- Modify: `Cargo.toml:14-45`
- Modify: `Cargo.lock`
- Modify: `src/ui/animation.rs`
- Test: `src/ui/animation.rs`

**Step 1: Add the narrowly configured dependency**

Add:

```toml
tachyonfx = { version = "0.25.1", default-features = false, features = ["std"] }
```

Do not enable DSL, WASM, or sendable support unless compilation proves a required
API is unavailable without it. Run `cargo check` once to update `Cargo.lock`.

**Step 2: Verify dependency compatibility**

Run: `cargo check`

Expected: PASS with the existing `ratatui 0.30.2`; no second incompatible Ratatui
widget type appears in compiler errors.

**Step 3: Write failing finite-effect lifecycle tests**

Cover:

- Starting an effect for an identity makes `has_active_effects()` true.
- Advancing beyond its duration removes it.
- Starting the same identity twice does not restart it.
- Replacing the identity cancels the old effect.
- Reduced/off modes do not enqueue TachyonFX effects.
- A zero-size area is ignored.

Use a tiny Ratatui `Buffer` and fixed durations; do not assert every intermediate
color value owned by TachyonFX.

**Step 4: Implement an internal effect manager wrapper**

Keep TachyonFX types private to `animation.rs`. Expose semantic methods:

- `start_overlay(identity, area)`
- `start_result(identity, area)`
- `cancel_rect_effects_on_resize(area)`
- `render_effects(frame_or_buffer)`
- `has_active_effects()`

Use TachyonFX `EffectManager` if its keyed replacement semantics fit these needs;
otherwise retain at most one overlay and one result `Effect`. Do not build a
general effect registry.

**Step 5: Define restrained effect recipes**

Overlay full-mode recipe:

```text
parallel(
  fade_from_fg(theme.muted, 160ms QuadOut),
  sweep_in(..., 160ms CubicOut)
)
```

Result full-mode recipe:

```text
sweep_in(accent -> existing colors, 240ms QuadOut)
```

Adapt exact constructors to TachyonFX 0.25.1 APIs. Filter effects to text/header
cells where possible and never use glitch, explode, bounce, elastic, or an
indefinite repeat.

**Step 6: Run focused tests and check**

Run:

```bash
cargo test ui::animation::tests --lib
cargo check
```

Expected: PASS.

### Task 9: Dim Backgrounds And Animate Overlay Entrances

**Files:**
- Modify: `src/ui/animation.rs`
- Modify: `src/ui/mod.rs:210-302,1762-2225`
- Modify: `src/ui/profiles.rs` if profile-manager bounds are not reported by `render_overlay`
- Modify: `src/ui/record_view.rs` if record-view bounds are not reported by `render_overlay`
- Test: `tests/ui_render.rs`

**Step 1: Write failing overlay identity tests**

In `animation.rs`, ensure identities are stable across internal selection/focus
changes. Examples:

- `TargetSelector { selected: 0 }` to `selected: 1` is the same overlay identity.
- `ExecutionConfirm` focus changes do not restart.
- Message title/body replacement is a new identity only if no stable ID exists;
  variant-only identity is acceptable if messages cannot replace in place.
- Closing then reopening starts a new entrance.

Use an explicit `OverlayIdentity` enum with stable IDs only where required. Do not
hash entire Overlay values because cursor/selection edits would restart effects.

**Step 2: Write failing dimming tests**

Render the same fixture with and without a Message overlay using fixed full,
reduced, and off times. Inspect the `TestBackend` buffer:

- Background workspace text remains present beneath the popup.
- Its foreground/background are dimmed outside the popup.
- Popup text retains normal contrast.
- Reduced/off final dimming is deterministic.
- Hit testing still resolves the popup's existing targets and does not add a new
  invisible input layer.

**Step 3: Write failing popup-area tests**

After rendering each major overlay family, assert `UiState` records a non-zero
`overlay_area` inside the terminal:

- Message/confirm popup.
- Help.
- Record View.
- Profile Manager.
- Target Selector/SQL Editor List.

Resize between frames and verify the recorded area changes without retaining the
old effect bounds.

**Step 4: Run focused tests and verify failure**

Run:

```bash
cargo test --test ui_render overlay_background_is_dimmed
cargo test --test ui_render overlay_reports_animation_area
```

Expected: FAIL because background dimming and overlay area reporting do not exist.

**Step 5: Add overlay area reporting**

Add `overlay_area: Option<Rect>` to `UiState` and reset it with the other
render-derived fields each frame. Change `render_overlay` and large overlay
renderers to return or assign the exact final popup area. Prefer returning `Rect`
from render helpers over duplicating layout calculations in the animation module.

**Step 6: Apply background dimming before popup rendering**

After workspace rendering and before `render_overlay`, modify only cells outside
the future popup if its bounds are known before drawing. If bounds are known only
after drawing, split layout calculation from content rendering so the area is
available first. Do not dim after drawing the popup.

For full mode, let the animation state interpolate color toward the final dim for
approximately 120 ms. Reduced/off apply the final dim immediately. Preserve the
original symbols and modifiers required for readability.

**Step 7: Start and render the overlay entrance effect**

After popup content is in the frame buffer, start the semantic overlay effect only
when `OverlayIdentity` changes, then apply it to `overlay_area`. Render effects as
the final UI step. Completion popups remain outside this path.

**Step 8: Handle resize and overlay replacement**

When terminal area or popup area changes, cancel the old rectangle-bound effect.
The popup must immediately be usable and fully rendered even if no replacement
effect is created. Closing an overlay clears its effect without delaying App state.

**Step 9: Run focused overlay tests**

Run:

```bash
cargo test --test ui_render overlay_background_is_dimmed
cargo test --test ui_render overlay_reports_animation_area
cargo test --test ui_render overlay
```

Expected: PASS.

### Task 10: Animate New Result Readiness Once

**Files:**
- Modify: `src/ui/animation.rs`
- Modify: `src/ui/mod.rs:1535-1620`
- Modify: `src/ui/relation.rs:99-329`
- Modify: `tests/ui_render.rs`
- Test: `src/ui/animation.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing result identity tests**

Define compact stable result identities:

```rust
pub(crate) enum ResultIdentity {
    Query { tab_id: Uuid, generation: u64 },
    Derived { tab_id: Uuid, generation: u64 },
    Relation(RelationRequest),
}
```

Test that a newly observed Ready identity starts one effect, repeated renders do
not restart it, failure/cancellation do not start it, and reduced/off modes skip
it.

**Step 2: Write failing buffer invariants**

Render at the start, midpoint, and completion of the result effect. Assert:

- Database cell symbols are unchanged at completion.
- Selection and result-cell hit regions are unchanged throughout.
- The effect stays inside the result panel.
- A second ordinary redraw after completion is identical to the static result.

Avoid snapshots of exact intermediate RGB values. Assert area containment and
that at least one intended header/accent cell changes in full mode.

**Step 3: Run tests and verify failure**

Run:

```bash
cargo test ui::animation::tests --lib result
cargo test --test ui_render result_ready
```

Expected: FAIL because result effects are not connected to rendered areas.

**Step 4: Report result panel areas**

Add a render-derived result area field to `UiState`, keyed by the active result
identity if needed. Set it only when a real SQL or relation result table/DDL body
is rendered. Skeletons do not report Ready areas.

**Step 5: Start the one-shot effect after real content renders**

When observation has queued a new result identity and its area is available,
start the full-mode result recipe and consume the queued transition. Filter the
effect to title/header/status text when possible; if a coarse rectangle is needed,
use foreground-only transformation so cell symbols remain intact.

**Step 6: Run focused tests**

Run:

```bash
cargo test ui::animation::tests --lib result
cargo test --test ui_render result_ready
```

Expected: PASS.

### Task 11: Complete Accessibility, Regression, And Performance Verification

**Files:**
- Modify: `README.md` only if global CLI options are documented there
- Modify: `docs/` CLI/user configuration documentation if an existing relevant file is found
- Modify: implementation files only for defects found during verification

**Step 1: Document the user-facing option**

Document:

```text
--motion <full|reduced|off>
```

Explain that reduced keeps low-frequency activity feedback and off uses static
loading feedback. Do not claim config-file persistence.

**Step 2: Run formatting**

Run: `cargo fmt --check`

Expected: PASS. If it fails, run `cargo fmt`, inspect the diff to ensure only
intended Rust files changed, then rerun `cargo fmt --check`.

**Step 3: Run focused UI and CLI tests**

Run:

```bash
cargo test cli::tests --lib
cargo test ui::animation::tests --lib
cargo test ui::loading::tests --lib
cargo test --test ui_render
```

Expected: PASS.

**Step 4: Run behavior regression suites**

Run:

```bash
cargo test --test app_flow
cargo test --test relation_tabs
cargo test --test relation_runtime
cargo test --test mouse
cargo test --test keymap
```

Expected: PASS; loading visuals must not alter cancellation, navigation, or hit
testing.

**Step 5: Run the full test suite**

Run: `cargo test`

Expected: PASS.

**Step 6: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Step 7: Manually verify terminal behavior**

Run each mode against a local SQLite profile and verify:

```bash
cargo run -- --icons ascii --motion full
cargo run -- --icons ascii --motion reduced
cargo run -- --icons ascii --motion off
```

Acceptance checklist:

- A sub-150 ms query does not flash a skeleton.
- A slow first query shows a non-selectable skeleton and cancel hint.
- Refreshing existing data keeps it readable and clearly labels it as previous.
- Relation Data and DDL show activity and remain cancellable.
- Popup entrance lasts no more than about 180 ms and never blocks keys.
- Esc closes immediately.
- Resize leaves no stale popup or effect cells.
- Full mode animates, reduced is low motion, and off is static.
- ASCII mode emits no non-ASCII animation or skeleton symbols.
- Leaving the app idle causes no continuous terminal draws or visible CPU use.

**Step 8: Inspect the final diff**

Run:

```bash
git status --short
git diff -- Cargo.toml Cargo.lock src/cli.rs src/runtime.rs src/ui tests/ui_render.rs README.md docs
```

Expected: only intended motion/loading files and any pre-existing user changes are
present. Do not revert or overwrite unrelated dirty-worktree changes.

## Definition Of Done

- `--motion full|reduced|off` parses and defaults to full.
- SQL, derived, relation Data, and relation DDL loads always have textual feedback.
- First loads show a delayed, non-interactive skeleton; refreshes preserve old data.
- Full mode uses short TachyonFX popup and result effects only in bounded areas.
- Reduced/off modes remain useful and deterministic.
- Overlay closing, cancellation, keyboard input, mouse hit regions, and resize stay immediate.
- The runtime avoids terminal draws when idle and redraws all active loading/effect states when needed.
- Focused tests, full `cargo test`, formatting, and Clippy all pass.
