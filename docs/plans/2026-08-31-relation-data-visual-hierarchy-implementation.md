# Relation Data Visual Hierarchy Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make relation view tabs, query inputs, and grid headers visually distinct while moving relation filters inside the `RELATION DATA` panel and preserving all existing interactions.

**Architecture:** Keep query state and actions unchanged. Extend the existing icon abstraction and shared query-bar renderer, compose the relation panel around that shared renderer, and adjust the shared data-grid geometry for a dedicated header divider row. Validate presentation through `TestBackend` render assertions and focused unit tests before running the full suite.

**Tech Stack:** Rust, Ratatui, `unicode-width`, Cargo integration tests.

---

### Task 1: Add query control icons

**Files:**
- Modify: `src/ui/icons.rs`
- Test: `src/ui/icons.rs`

**Step 1: Write failing icon mapping tests**

Add tests that iterate over `IconMode::NerdFont`, `IconMode::Unicode`, and `IconMode::Ascii`, asserting that filter and sort mappings are non-empty, contain no terminal controls, and use no private-use characters outside Nerd Font mode.

**Step 2: Run the focused test**

Run: `cargo test ui::icons::tests --lib`

Expected: FAIL because query filter and sort icon methods do not exist.

**Step 3: Add minimal icon mappings**

Add `IconSet::query_filter()` and `IconSet::query_sort()` methods. Use Material Design Nerd Font constants for Nerd Font mode, readable single-cell Unicode symbols where possible, and short fixed-width ASCII fallbacks such as `F` and `S`.

**Step 4: Re-run the focused test**

Run: `cargo test ui::icons::tests --lib`

Expected: PASS.

### Task 2: Redesign the shared query bar

**Files:**
- Modify: `src/ui/query_bar.rs`
- Modify: `src/ui/relation.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/ui_render.rs`

**Step 1: Add failing shared-query-bar render tests**

Add SQL and relation render assertions for:

- Filter and sort labels/icons.
- Persistent underline slots when values are empty.
- Accent styling on the focused field.
- Safe output in Nerd Font, Unicode, and ASCII icon modes.
- Horizontal fields at normal width and stacked fields at constrained width.

Use buffer cell style assertions where plain text output cannot prove focus color or underline treatment.

**Step 2: Run the focused tests**

Run: `cargo test --test ui_render query_bar -- --nocapture`

Expected: FAIL because the current query bar has no icons, persistent underlines, or responsive stacking.

**Step 3: Implement query-bar geometry and styling**

Change `query_bar::render` to receive the active `IconSet`. Compute field layout from available width: two columns when both fields retain a meaningful input slot, otherwise two vertical rows. Render each field as icon, label, projected value, and a fill of underline characters through the remaining cells.

Keep existing behavior intact:

- `DataQueryCapability` controls interactivity.
- Existing `HitTarget::DataQueryInput` regions follow the new field rectangles.
- Focused input still calls `render_text_input` or an equivalent projection path and exposes the bar cursor position.
- Completion popup anchoring receives the actual focused cursor.
- Errors render below the field rows.

Return layout height information or provide a helper so callers can reserve exactly the required rows without duplicating breakpoint logic.

**Step 4: Update both callers**

Pass `state.activity_icons` from SQL results and relation results. Replace fixed query heights with the shared layout-height calculation, including the optional error row.

**Step 5: Re-run focused tests**

Run: `cargo test --test ui_render query_bar -- --nocapture`

Expected: PASS.

### Task 3: Make DATA and DDL explicit secondary tabs

**Files:**
- Modify: `src/ui/relation.rs:22-73`
- Test: `tests/ui_render.rs:808-848`

**Step 1: Strengthen the relation-tab render test**

Update `relation_page_renders_data_ddl_selectors_and_relation_layout` to assert:

- `DATA` and `DDL` remain present and clickable.
- The relation title is not rendered on the secondary-tab row.
- The active tab has an underline in the row below its label.
- The active underline and click target align with the rendered label.
- The secondary active state does not use the filled accent background used by workspace tabs.

**Step 2: Run the focused test**

Run: `cargo test --test ui_render relation_page_renders_data_ddl_selectors_and_relation_layout -- --nocapture`

Expected: FAIL because the relation title is still present and tabs have no underline.

**Step 3: Implement the secondary tab rail**

Render a two-row tab rail: labels on the first row and a short active underline on the second. Remove `tab.descriptor.title` from this local navigation. Keep muted inactive text and accent bold active text without a filled background. Derive hit-region widths from actual Unicode cell widths rather than fixed constants.

**Step 4: Re-run the focused test**

Run: `cargo test --test ui_render relation_page_renders_data_ddl_selectors_and_relation_layout -- --nocapture`

Expected: PASS at 80x24, 120x36, and 180x50.

### Task 4: Move relation filters inside RELATION DATA

**Files:**
- Modify: `src/ui/relation.rs:98-265`
- Test: `tests/ui_render.rs`

**Step 1: Add failing hierarchy tests**

For ready, empty, loading-without-previous, loading-with-previous, failed, and cancelled relation states, assert that:

- The `RELATION DATA` border begins before the query fields.
- Query fields are inside the panel's horizontal bounds.
- Query fields appear before the grid header or state content.
- There is only one enclosing panel border, with no nested filter box.

**Step 2: Run relation render tests**

Run: `cargo test --test ui_render relation -- --nocapture`

Expected: FAIL because query fields currently render above the panel block.

**Step 3: Refactor relation panel composition**

Create the `RELATION DATA` block first, calculate its inner rectangle, then split that inner rectangle into query controls, optional status/error content, grid/state body, and footer. Pass a borderless/default block to the data grid if the outer relation block has already been rendered, avoiding nested borders.

For loading skeletons, preserve one outer `RELATION DATA` block and render skeleton content only within its inner body. Keep query completion anchored to the relation viewport and focused input cursor.

**Step 4: Re-run relation render tests**

Run: `cargo test --test ui_render relation -- --nocapture`

Expected: PASS.

### Task 5: Replace the grid header fill with an emphasized divider

**Files:**
- Modify: `src/ui/data_grid.rs`
- Test: `src/ui/data_grid.rs`
- Test: `tests/ui_render.rs`

**Step 1: Add failing grid geometry and style tests**

Add assertions that:

- Header cells use `theme.surface`, not `theme.grid_header`.
- Column labels are bold while `#` remains muted.
- A horizontal divider occupies the row directly below the header.
- First data-row hit regions move below the divider.
- Visible-row calculations account for the divider and scrollbar.
- Empty-state text remains immediately below the divider and inside the grid bounds.

**Step 2: Run focused grid tests**

Run: `cargo test ui::data_grid --lib && cargo test --test ui_render empty_relation_preview_renders_clean_empty_state -- --nocapture`

Expected: FAIL because the header uses `grid_header` and no dedicated divider row exists.

**Step 3: Implement the divider without changing selection semantics**

Render header cells on `theme.surface`, with bold text labels, a muted row-number header, and low-emphasis vertical separators. Reserve one row below the Ratatui table header for a horizontal divider, or render the table body with an offset that produces the same geometry. Update all linked constants and calculations together:

- `visible_rows`
- `row_y`
- cell hit regions
- resize hit regions
- empty-state Y position
- scrollbar body geometry
- viewport metrics

Use `grid_border` for the divider and intersections. Keep row, cell, edit-state, and scrollbar styles unchanged.

**Step 4: Re-run focused grid tests**

Run: `cargo test ui::data_grid --lib && cargo test --test ui_render empty_relation_preview_renders_clean_empty_state -- --nocapture`

Expected: PASS.

### Task 6: Full regression and formatting

**Files:**
- Verify: `src/ui/icons.rs`
- Verify: `src/ui/query_bar.rs`
- Verify: `src/ui/relation.rs`
- Verify: `src/ui/data_grid.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Format**

Run: `cargo fmt --all -- --check`

If it fails, run `cargo fmt --all`, inspect the diff, then re-run the check.

**Step 2: Run focused integration suites**

Run: `cargo test --test ui_render --test relation_tabs --test mouse`

Expected: PASS.

**Step 3: Run lint**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS.

**Step 4: Run the full test suite**

Run: `cargo test --all-targets --all-features`

Expected: PASS.

**Step 5: Inspect final changes**

Run: `git status --short` and `git diff --check`.

Expected: only the intended design document, implementation plan, UI source files, and tests are changed; no whitespace errors are reported.

No commits are included in this plan because repository commits require explicit user approval in this environment.
