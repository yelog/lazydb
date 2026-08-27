# Profile URL Section Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the visible URL Format field with a fixed bottom URL configuration section containing driver-specific examples while preserving URL and structured-field synchronization.

**Architecture:** Keep URL format as internal draft state, but remove `UrlFormat` and `Url` from the scrollable structured field area. Render URL separately in a fixed section above messages/buttons, reserving adaptive example rows based on available form height.

**Tech Stack:** Rust 2024, Ratatui 0.30, existing ProfileDraft/ProfileManagerState model and UI integration tests.

---

### Task 1: Update Visible Field Order Tests

**Files:**
- Modify: `tests/profile_draft.rs` near visible field and navigation tests
- Modify: `tests/keymap.rs` if it asserts `UrlFormat` navigation

**Step 1: Change expected visible field arrays**

Update assertions so `ProfileField::UrlFormat` is absent and `ProfileField::Url` appears after the last structured field and before action buttons for Postgres, MySQL, SQLite file, and SQLite memory drafts.

**Step 2: Add navigation coverage**

Verify that forward navigation reaches URL after the final structured field and then reaches Test. Verify backward navigation returns from URL to the final structured field. Confirm that `UrlFormat` is never selected.

**Step 3: Run focused tests before implementation**

Run:

```bash
cargo test --test profile_draft visible_fields -- --nocapture
cargo test --test keymap profile_form -- --nocapture
```

Expected: updated ordering assertions fail before the field arrays are changed.

### Task 2: Remove URL Format From Visible Arrays

**Files:**
- Modify: `src/model/profile_manager.rs:1966-2036`

**Step 1: Reorder each Driver field array**

For each field array:

- Remove `ProfileField::UrlFormat`.
- Move `ProfileField::Url` after the last structured field.
- Keep URL before Test, Save, SaveAndConnect, and Cancel.
- Update array lengths.

Do not remove the enum variant, draft state, cycle branch, parser format assignment, persistence format, or `refresh_url` behavior.

**Step 2: Run model and keymap tests**

Run:

```bash
cargo test --test profile_draft
cargo test --test keymap
```

Expected: all tests pass after updating relevant expectations.

### Task 3: Add Fixed URL Section Rendering Tests

**Files:**
- Modify: `tests/ui_render.rs` near profile form tests

**Step 1: Assert URL Format is absent**

Render new and edit forms and assert they do not contain `URL FORMAT`.

**Step 2: Assert Driver-specific examples**

Render each Driver and assert its supported examples are shown while examples from other Drivers are absent.

**Step 3: Assert compact behavior**

At the existing compact form size, assert URL, at least the first Driver example, buttons, and hints remain visible.

**Step 4: Assert examples are not interactive**

Verify hit regions include the URL field but do not add any new example target. Existing `HitTarget` variants should remain unchanged.

**Step 5: Run focused UI tests before implementation**

Run:

```bash
cargo test --test ui_render profile_url -- --nocapture
cargo test --test ui_render profile_form_remains_actionable_in_compact_layout -- --nocapture
```

Expected: new layout assertions fail before implementation.

### Task 4: Split Structured and URL Rendering

**Files:**
- Modify: `src/ui/profiles.rs:44-173`

**Step 1: Separate field collections**

Build the scrollable structured field collection by excluding button fields, `UrlFormat`, and `Url`. Keep URL in the model's visible field list for navigation.

**Step 2: Reserve fixed URL section height**

Calculate available rows above the message/buttons area. Reserve one separator row and one URL row first, then reserve as many driver example rows as fit, with at least one example at supported form sizes. Use the remainder as structured field row capacity.

**Step 3: Render the structured viewport**

Calculate viewport start only from the selected structured field. When the selected field is URL or an action button, do not force the structured field viewport toward an unrelated index; keep a stable bounded start appropriate to the end of the structured list.

**Step 4: Render URL using the existing field path**

Call `render_field` with `ProfileField::Url` in the fixed URL row. Add its existing `ProfileField` hit region when not busy and call `render_field_cursor` when selected.

**Step 5: Render static examples**

Add a small helper returning `&'static [&'static str]` for each `DatabaseKind`. Render only the number that fits, using muted/dim styling and a first-row `EXAMPLES` marker if space allows. Do not create hit regions.

### Task 5: Preserve URL Commit and Synchronization

**Files:**
- Verify: `src/model/profile_manager.rs:494-531,1503-1513,1576-1591,1661-1670`
- Verify: `tests/profile_draft.rs`
- Verify: `tests/profile_reducer.rs`
- Verify: `tests/profile_url.rs`

**Step 1: Run URL behavior tests**

Run:

```bash
cargo test --test profile_url
cargo test --test profile_draft url -- --nocapture
cargo test --test profile_reducer url -- --nocapture
```

Expected: URL parsing, format retention, structured regeneration, invalid URL focus retention, and password handling all pass without implementation changes to those paths.

### Task 6: Full Verification

**Files:**
- Verify: `src/model/profile_manager.rs`
- Verify: `src/ui/profiles.rs`
- Verify: profile and UI tests

**Step 1: Run formatting and compile checks**

Run:

```bash
cargo fmt --check
cargo check --all-targets
```

Expected: both pass.

**Step 2: Run complete tests**

Run:

```bash
cargo test --all-targets
```

Expected: all tests pass.

**Step 3: Inspect the diff**

Run:

```bash
git diff --check
```

Expected: the diff changes visible field order, fixed URL section rendering, examples, and focused test expectations only. Internal URL format and synchronization logic remain intact.

**Step 4: Manual smoke test**

In new/edit forms, verify each Driver at normal and compact terminal sizes. Confirm URL Format is absent, URL stays above the buttons with a blank separator, examples follow Driver changes, Tab navigation reaches URL after structured fields, URL import updates structured fields, structured edits update URL, and invalid URLs retain URL focus with an error.
