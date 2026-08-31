# Record View Field Selection Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add visible, bounded field selection to Record View so `j/k` works even when every field fits in the viewport.

**Architecture:** Extend `RecordViewState` with a selected field index and keep `field_offset` as viewport-only state. Model methods update both values to keep the selection visible; the renderer highlights the selected full row using the existing theme.

**Tech Stack:** Rust 2024, Ratatui 0.30, existing LazyDB App/Overlay/UI test architecture

---

### Task 1: Model Field Selection

**Files:**
- Modify: `src/model/record_view.rs`
- Test: `src/model/record_view.rs`

**Step 1: Write failing tests**

Add tests proving that one-line movement changes `selected_field` when all fields
fit, clamps at both bounds, follows the selection with `field_offset`, and clamps
both values when the field count shrinks.

**Step 2: Run the focused model tests**

Run: `cargo test model::record_view::tests --lib`

Expected: FAIL because `selected_field` and selection-aware navigation do not
exist.

**Step 3: Implement selection-aware state**

Add `selected_field: usize`. Change movement and jump methods to select a field,
then adjust `field_offset` only enough to keep it inside the current viewport.
Reset both values for zero fields and clamp with saturating arithmetic.

**Step 4: Run the focused model tests**

Run: `cargo test model::record_view::tests --lib`

Expected: PASS.

### Task 2: Render The Selected Field

**Files:**
- Modify: `src/ui/record_view.rs`
- Test: `tests/ui_render.rs`

**Step 1: Add a failing style assertion**

Render Record View, inspect the Ratatui buffer, and assert that the first field
row uses the theme selection background. Move down one field and assert that the
selection background moves to the second field row.

**Step 2: Run the focused UI tests**

Run: `cargo test --test ui_render record_view`

Expected: FAIL because all field rows currently use the raised-surface
background.

**Step 3: Implement full-row highlighting**

For the selected field line, apply `theme.selection` as its background and use
readable selected text colors across field, type, and value spans. Preserve NULL
italic styling and warning semantics where they remain legible.

**Step 4: Run focused UI and app tests**

Run: `cargo test --test ui_render record_view`

Run: `cargo test app::tests::record_view --lib`

Expected: PASS.

### Task 3: Regression Verification

**Files:**
- Verify: `src/model/record_view.rs`
- Verify: `src/ui/record_view.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Format and check whitespace**

Run: `cargo fmt --all -- --check`

Run: `git diff --check`

Expected: PASS.

**Step 2: Run Record View keymap tests**

Run: `cargo test --test keymap record_view`

Expected: PASS; key bindings retain their current semantic actions.

**Step 3: Run Clippy for the changed targets**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS with no warnings.
