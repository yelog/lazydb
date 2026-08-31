# Visible Objects State And Loading Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace Visible Objects checkbox text with mode-aware semantic icons and show a non-destructive, interaction-safe loading state during catalog discovery.

**Architecture:** Extend the existing `IconSet` with tri-state selection mappings and render each scope row as independently styled icon and label spans. Reuse `ProfileManagerState::scope_discovery_request` as the business loading state, expose it to the existing UI animation observation as a stable load identity, and render the shared `ActivityIndicator` above preserved rows. Block stale selection edits in the model and suppress toggle/refresh actions and mouse targets while loading.

**Tech Stack:** Rust, Ratatui 0.30, `nerd-font-symbols`, Crossterm input mapping, existing UI animation/loading modules, Rust integration tests.

---

### Task 1: Add Mode-Aware Scope Selection Icons

**Files:**
- Modify: `src/ui/icons.rs:2-3,18-158`
- Test: `src/ui/icons.rs:160-237`

**Step 1: Write failing icon mapping tests**

Add a UI-local selection icon enum and tests for all modes. Keep this presentation

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionIcon {
    Unchecked,
    Checked,
    Partial,
}

#[test]
fn scope_selection_icons_match_each_mode() {
    assert_eq!(
        IconSet::new(IconMode::NerdFont).selection(SelectionIcon::Unchecked),
        md::MD_CHECKBOX_BLANK_OUTLINE
    );
    assert_eq!(
        IconSet::new(IconMode::NerdFont).selection(SelectionIcon::Checked),
        md::MD_CHECKBOX_MARKED
    );
    assert_eq!(
        IconSet::new(IconMode::NerdFont).selection(SelectionIcon::Partial),
        md::MD_CHECKBOX_INTERMEDIATE
    );
    assert_eq!(IconSet::new(IconMode::Unicode).selection(SelectionIcon::Unchecked), "☐");
    assert_eq!(IconSet::new(IconMode::Unicode).selection(SelectionIcon::Checked), "☑");
    assert_eq!(IconSet::new(IconMode::Unicode).selection(SelectionIcon::Partial), "▣");
    assert_eq!(IconSet::new(IconMode::Ascii).selection(SelectionIcon::Unchecked), "[ ]");
    assert_eq!(IconSet::new(IconMode::Ascii).selection(SelectionIcon::Checked), "[x]");
    assert_eq!(IconSet::new(IconMode::Ascii).selection(SelectionIcon::Partial), "[-]");
}
```

Extend `every_mode_has_safe_mappings` to iterate over all three `SelectionIcon`
values. Assert non-empty/control-character-free output, ASCII-only output in ASCII
mode, and no private-use characters in Unicode mode.

**Step 2: Run the icon tests and verify failure**

Run:

```bash
cargo test ui::icons::tests --lib
```

Expected: compilation fails because `SelectionIcon` and `IconSet::selection` do not
exist.

**Step 3: Implement the minimal mappings**

Import the existing MDI module and add:

```rust
pub(crate) const fn selection(self, state: SelectionIcon) -> &'static str {
    match (self.mode, state) {
        (IconMode::NerdFont, SelectionIcon::Unchecked) => md::MD_CHECKBOX_BLANK_OUTLINE,
        (IconMode::NerdFont, SelectionIcon::Checked) => md::MD_CHECKBOX_MARKED,
        (IconMode::NerdFont, SelectionIcon::Partial) => md::MD_CHECKBOX_INTERMEDIATE,
        (IconMode::Unicode, SelectionIcon::Unchecked) => "☐",
        (IconMode::Unicode, SelectionIcon::Checked) => "☑",
        (IconMode::Unicode, SelectionIcon::Partial) => "▣",
        (IconMode::Ascii, SelectionIcon::Unchecked) => "[ ]",
        (IconMode::Ascii, SelectionIcon::Checked) => "[x]",
        (IconMode::Ascii, SelectionIcon::Partial) => "[-]",
    }
}
```

Use `nerd_font_symbols::md` constants instead of embedding Nerd Font private-use
characters directly.

**Step 4: Run the icon tests**

Run:

```bash
cargo test ui::icons::tests --lib
```

Expected: all icon tests pass.

**Step 5: Commit the task when explicitly requested**

Do not commit automatically. If requested, stage only `src/ui/icons.rs` and use:

```bash
git commit -m "feat(ui): add visible object state icons"
```

### Task 2: Track Scope Discovery In The Existing Animation State

**Files:**
- Modify: `src/ui/animation.rs:14-19`
- Modify: `src/ui/mod.rs:178-185,385-437`
- Test: `src/ui/animation.rs:221-363`

**Step 1: Write a failing animation-observation test**

Add a test around `animation_observation` or the closest existing UI observation

```rust
LoadIdentity::ProfileScope { request_id }
```

The identity must disappear after `finish_scope_discovery`. Use the request id,
which already changes for every refresh, rather than hashing credentials or copying
the discovery fingerprint into UI state.

**Step 2: Run the focused test and verify failure**

Run:

```bash
cargo test ui::animation --lib
```

Expected: compilation or assertion failure because profile scope discovery is not
part of `LoadIdentity` or `animation_observation`.

**Step 3: Add the profile-scope load identity**

Extend the enum:

```rust
ProfileScope { request_id: u64 },
```

In `animation_observation`, inspect `app.profile_manager` before the early return for
a missing workspace tab. If `scope_discovery_request` is `Some((request_id, _))`,
insert the corresponding identity. Then retain the current SQL/relation observation
logic unchanged.

Add a narrow `UiState` helper so `profiles.rs` does not reach through animation
internals:

```rust
pub(crate) fn profile_scope_loading_presentation(
    &self,
    request_id: u64,
) -> (MotionMode, Duration) {
    let identity = animation::LoadIdentity::ProfileScope { request_id };
    (
        self.animations.mode(),
        self.animations.elapsed(&identity).unwrap_or_default(),
    )
}
```

Import `Duration` in `src/ui/mod.rs` if required. Keep timers out of `App` and
`ProfileManagerState`.

**Step 4: Run animation and UI library tests**

Run:

```bash
cargo test ui::animation --lib
```

Expected: all tests pass and pending profile discovery participates in demand-driven
spinner redraws.

**Step 5: Commit the task when explicitly requested**

Do not commit automatically. If requested, stage only `src/ui/animation.rs` and
`src/ui/mod.rs`, then use:

```bash
git commit -m "feat(ui): animate visible object discovery"
```

### Task 3: Render Tri-State Rows And Preserved Loading Content

**Files:**
- Modify: `src/ui/profiles.rs:1-23,25-43,282-351`
- Test: `tests/ui_render.rs:268-303,2000-2308`

**Step 1: Add failing tri-state render tests**

Create a profile-manager Scope fixture with discovered database/schema rows covering
`Unchecked`, `Checked`, and `Partial`. Render it once for every `IconMode` and assert
that each output contains the matching value returned by
`IconSet::selection(SelectionIcon::...)`.

Add buffer assertions for the Nerd Font/default render:

- Checked icon foreground is `Theme::default().accent`.
- Partial icon foreground is `Theme::default().warning`.
- Unchecked icon foreground is `Theme::default().muted`.
- The active row background remains `Theme::default().selection` across the icon
  and label cells.

Locate cells through rendered row text/hit-region geometry instead of fixed absolute
screen coordinates.

**Step 2: Add failing loading render tests**

Start discovery through `Action::ProfileOpenScope` so the test uses the real pending
state. Assert:

```rust
assert!(output.contains("Loading visible objects"));
assert!(output.contains("discovering databases and schemas"));
assert!(output.contains("Loading..."));
assert!(!output.contains("Space toggle"));
assert!(!output.contains("r refresh"));
assert!(state.hit_regions.iter().all(|region| {
    !matches!(region.target, HitTarget::ProfileScopeRow(_))
}));
```

Use a saved scope fixture to prove its database/schema names remain visible. Add a
second no-row fixture and assert `Waiting for catalog discovery...` appears.

**Step 3: Run UI tests and verify failure**

Run:

```bash
cargo test --test ui_render visible_objects -- --nocapture
```

Expected: tests fail because the renderer still emits `[ ]`, `[x]`, and `[-]`, does
not render `ActivityIndicator`, and still creates loading hit regions.

**Step 4: Pass `IconSet` into the Scope renderer**

Change the page dispatch from:

```rust
ProfileManagerPage::Scope => render_scope(frame, area, manager, state, theme),
```

to pass `icons`, and update `render_scope` accordingly.

Import:

```rust
use super::{
    HitRegion, HitTarget, ProfileButton, Theme, UiState,
    icons::{IconSet, SelectionIcon},
    loading::ActivityIndicator,
};
```

**Step 5: Render loading before rows**

Derive loading from `manager.scope_discovery_request`. When present, reserve the
first content line and render:

```rust
ActivityIndicator {
    mode,
    icons,
    elapsed,
    label: "Loading visible objects",
    detail: Some("discovering databases and schemas"),
    cancellable: false,
    style: Style::new().fg(theme.action).bg(theme.surface),
}
```

Start rows one line lower while loading and reduce their `take(...)` capacity by one.
When rows are empty, render `Waiting for catalog discovery...` in `theme.muted` on
the next content line.

Do not render `scope_warning` while loading because
`begin_scope_discovery` currently stores `Discovering databases and schemas...` in
that slot and the ActivityIndicator supersedes it. Continue rendering warning/error
text after completion.

**Step 6: Render icons and labels as separate spans**

Map model states locally:

```rust
let (icon_state, icon_color) = match row.selection {
    ScopeSelectionState::Unchecked => (SelectionIcon::Unchecked, theme.muted),
    ScopeSelectionState::Checked => (SelectionIcon::Checked, theme.accent),
    ScopeSelectionState::Partial => (SelectionIcon::Partial, theme.warning),
};
```

Build `Line::from` with separate icon, separator, name, and optional mirrored suffix
spans. Use `theme.selection` for every span background on the active row. While
loading, use `theme.muted` for available labels; unavailable labels remain warning
colored. Continue to sanitize names through the existing row-generation/safe-text
path; do not interpolate unsanitized database output into control sequences.

Only push `HitTarget::ProfileScopeRow` regions when not loading.

**Step 7: Switch the hint by loading state**

Render:

```rust
if loading {
    "Loading...   Enter back   Esc back"
} else {
    "Space toggle   r refresh   Enter back   Esc back"
}
```

**Step 8: Run focused UI tests**

Run:

```bash
cargo test --test ui_render visible_objects -- --nocapture
cargo test --test ui_render profile_manager -- --nocapture
```

Expected: tri-state, color, loading, preserved-row, no-row, hit-region, and existing
profile-manager tests pass.

**Step 9: Commit the task when explicitly requested**

Do not commit automatically. If requested, stage only `src/ui/profiles.rs` and the
new `tests/ui_render.rs` hunks, then use:

```bash
git commit -m "feat(ui): improve visible object picker feedback"
```

### Task 4: Block Scope Mutation And Duplicate Refresh While Loading

**Files:**
- Modify: `src/model/profile_manager.rs:1235-1237,1287-1315`
- Modify: `src/input/keymap.rs:1052-1064`
- Test: `tests/profile_draft.rs:92-175`
- Test: `tests/profile_reducer.rs:203-270`
- Test: `tests/keymap.rs:500-536`

**Step 1: Add a failing model guard test**

In `tests/profile_draft.rs`, start from a discovered selectable scope, call
`begin_scope_discovery` with its current fingerprint, clone the catalog scope, and
assert:

```rust
assert!(!state.toggle_scope_row("database:moss_biz"));
assert_eq!(state.draft.as_ref().unwrap().catalog_scope, before);
```

**Step 2: Add failing reducer and keymap tests**

In `tests/profile_reducer.rs`, while the initial discovery is pending:

- Dispatch `ProfileToggleScopeRow` and verify the saved scope is unchanged.
- Dispatch `ProfileRefreshScope` and verify it returns no command and does not
  replace the pending request id.
- Dispatch `ProfileScopeBack` and verify the page returns to Form.

In `tests/keymap.rs`, update the Scope loading expectations:

- Space maps to `None`.
- `r` maps to `None`.
- Up/down and `j`/`k` still map to movement.
- Enter and Esc still map to `ProfileScopeBack`.

After dispatching a successful discovery response, verify Space and `r` map to their
normal actions again.

**Step 3: Run focused tests and verify failure**

Run:

```bash
cargo test --test profile_draft toggle_scope_row_is_blocked_while_discovery_is_loading
cargo test --test profile_reducer visible_objects_loading
cargo test --test keymap profile_scope
```

Expected: the model currently allows toggles and the keymap currently emits toggle
and refresh actions while loading.

**Step 4: Add the model guard**

At the start of `toggle_scope_row`:

```rust
if self.scope_discovery_loading() {
    return false;
}
```

Keep `open_profile_scope`'s existing `scope_discovery_loading()` early return as the
reducer-level duplicate refresh guard.

**Step 5: Suppress loading-only key actions**

In the Scope branch of `map_profile_manager`, map `r` and Space only when
`!manager.scope_discovery_loading()`. Preserve navigation and back mappings
regardless of loading state.

No `src/input/mouse.rs` change is required: the renderer omits scope-row hit regions
while loading, and the model guard protects direct/stale actions. Avoid touching the
existing unrelated worktree changes in `src/input/mouse.rs`.

**Step 6: Run focused model, reducer, and keymap tests**

Run:

```bash
cargo test --test profile_draft
cargo test --test profile_reducer
cargo test --test keymap
```

Expected: all tests pass.

**Step 7: Commit the task when explicitly requested**

Do not commit automatically. If requested, stage only the task's files/hunks and
use:

```bash
git commit -m "fix(profile): lock visible scope during discovery"
```

### Task 5: Verify Failure Recovery And Full Quality Gates

**Files:**
- Test: `tests/profile_reducer.rs`
- Test: `tests/ui_render.rs`
- Modify only if required by failures: files changed in Tasks 1-4

**Step 1: Add a failure-recovery regression test if not already covered**

Start discovery with a saved scope, dispatch
`Action::ProfileCatalogDiscoveryFailed` using the matching request and fingerprint,
then assert:

- `scope_discovery_loading()` is false.
- Saved scope rows remain rendered.
- Output contains the sanitized `Catalog discovery failed` warning.
- Scope-row hit regions are restored.
- Normal toggle/refresh hint is restored.

Use terminal-control input in the failure message and assert it is sanitized rather
than emitted literally.

**Step 2: Run all feature-focused tests**

Run:

```bash
cargo test ui::icons::tests --lib
cargo test ui::animation --lib
cargo test --test profile_draft
cargo test --test profile_reducer
cargo test --test keymap
cargo test --test ui_render
cargo test --test mouse
```

Expected: all commands pass. The mouse suite is included because hit-region changes
affect mouse behavior even though `src/input/mouse.rs` should not need modification.

**Step 3: Format and inspect only intended changes**

Run:

```bash
cargo fmt --check
```

Expected: formatting passes; the diff contains only this feature. Do not revert or
stage pre-existing changes in `src/action.rs`, `src/app.rs`, `src/input/mouse.rs`, or
`tests/mouse.rs`. If implementation needs `src/app.rs`, edit only the precise scope
action hunks and preserve all concurrent changes.

**Step 4: Run the complete Rust suite**

Run:

```bash
cargo test
```

Expected: all tests pass.

**Step 5: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: no warnings or errors.

**Step 6: Commit the final verification changes when explicitly requested**

Do not commit automatically. If requested, inspect `git status`, `git diff`, and
recent history, stage only intended feature files, and use an appropriate concise
message such as:

```bash
git commit -m "test(ui): cover visible object loading recovery"
```
