# Profile Driver Icons Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add Explorer-consistent database icons to all Driver options in the new/edit connection form.

**Architecture:** Thread the existing `IconSet` from the top-level profile manager renderer through the form and field renderer into `render_driver_options`. Build each option label from `IconSet::database(kind)` plus `kind_name(kind)`, then use the complete label's terminal display width for layout and hit regions.

**Tech Stack:** Rust 2024, Ratatui 0.30, `unicode-width` via Ratatui's `CellWidth`, existing `IconSet` and UI integration tests.

---

### Task 1: Add Driver Icon Rendering Coverage

**Files:**
- Modify: `tests/ui_render.rs` near the existing Driver option rendering test

**Step 1: Identify the existing Driver selector fixture and assertions**

Use the existing profile form rendering test that checks individual Driver option targets and selected styling. Keep its setup and assertions intact, and add assertions against the rendered output for the three database labels/icons using the default `IconSet`.

**Step 2: Add alternate icon mode coverage if needed**

If the existing `explorer_uses_selected_icon_mode` test already exercises `IconSet::Unicode` and `IconSet::Ascii`, extend the Driver-specific test with those modes only when this is needed to distinguish option labels from Explorer output. Assert that the Driver labels contain `PG`, `MY`, and `SQ` in fallback modes.

**Step 3: Run the focused test before implementation**

Run:

```bash
cargo test --test ui_render driver_options -- --nocapture
```

Expected: the new icon-specific assertion fails before implementation, while existing Driver selector assertions pass.

### Task 2: Thread IconSet Through Profile Rendering

**Files:**
- Modify: `src/ui/mod.rs` profile manager call site
- Modify: `src/ui/profiles.rs:25-166,318-416`

**Step 1: Pass the active IconSet into the profile manager renderer**

Update the `render_profile_manager` call path and signatures so the same `icons::IconSet` selected for the rest of the UI reaches `render_form`, then `render_field`, and finally `render_driver_options`.

Do not construct a new icon set inside `profiles.rs`; use the caller's active mode.

**Step 2: Build complete Driver option labels**

In `render_driver_options`, replace the plain name label with a complete icon-and-name label:

```rust
let label = format!("{} {}", icons.database(kind), kind_name(kind));
let width = label.cell_width();
```

Render this label using the existing style and use the same width for the option `Rect` and `HitRegion`.

**Step 3: Preserve layout and interaction behavior**

Keep the existing one-cell spacing, fit check, selected/busy styles, and `ProfileDriver(kind)` hit target unchanged. The fit check must use the complete label width so icons cannot be rendered outside the hit region or overlap the next option.

### Task 3: Verify the Full Change

**Files:**
- Verify: `src/ui/mod.rs`
- Verify: `src/ui/profiles.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Run focused UI tests**

Run:

```bash
cargo test --test ui_render driver_options -- --nocapture
cargo test --test ui_render explorer_uses_selected_icon_mode -- --nocapture
```

Expected: all selected tests pass.

**Step 2: Run formatting and compile checks**

Run:

```bash
cargo fmt --check
cargo check --all-targets
```

Expected: both commands exit successfully.

**Step 3: Run the complete test suite**

Run:

```bash
cargo test --all-targets
```

Expected: all tests pass.

**Step 4: Inspect the final diff**

Run:

```bash
git diff --check
```

Expected: the diff only threads the existing icon set through profile rendering, renders complete icon-and-name Driver labels, and adds focused UI assertions.

**Step 5: Perform a manual terminal smoke test when available**

Open the new/edit connection form in Nerd Font, Unicode, and ASCII icon modes. Confirm that Postgres, MySQL, and SQLite each show an icon or fallback label, selection highlighting covers the entire option, and mouse targeting still selects the intended Driver.
