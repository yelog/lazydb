# Connection Form UI/UX Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Redesign the new/edit connection form into a compact, grouped, responsive, keyboard-first interface with clear focus, semantic feedback, and one visually dominant primary action.

**Architecture:** Keep the form as a centered single-column Ratatui modal so visual order, keyboard order, and mouse hit regions stay aligned. Build a small render-only layout model in `src/ui/profiles.rs`, derive section and status presentation from `ProfileManagerState`, and introduce typed form feedback so success, warning, and error states no longer share the same amber style. Preserve the existing profile persistence, validation, connection, and command flows.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm 0.29, existing reducer/action architecture, Rust integration tests with `TestBackend`.

---

## Product Decisions

This plan intentionally makes the following decisions before implementation:

1. Use the recommended compact centered single-column layout, not a wide two-column form. This keeps Tab order identical to visual order and avoids complex cursor/mouse geometry.
2. Cap the form at 106 columns and 34 rows. On larger terminals, center the modal instead of stretching field highlights across the screen.
3. Keep the outer panel border but do not add nested bordered cards. Sections use a restrained heading and whitespace only.
4. Render section headings only when enough vertical space is available. Compact terminals keep the same fields and actions without decorative rows.
5. Reorder profile fields so the data model's navigation order matches the new visual grouping.
6. Keep the generated URL editable, but hide persistent URL examples. Show one contextual protocol hint only while the URL field is focused.
7. Reserve a stable two-row feedback area above the actions so testing and validation do not move the action bar.
8. Make `Save & Connect` the only filled primary action. `Test` and `Save` are secondary; `Cancel` is tertiary.
9. Use natural casing for labels and values. Keep panel and section titles uppercase because they act as navigational landmarks.
10. Preserve all existing keyboard shortcuts, mouse targets, secret redaction, and busy-state input blocking.

## Acceptance Criteria

- At `160x40`, the connection panel is centered and no wider than 106 columns.
- At `120x36`, the form shows `CONNECTION`, `AUTHENTICATION`, `OPTIONS`, and `CONNECTION URL` sections without nested borders.
- At `80x24`, all fields remain reachable through the existing viewport behavior, the selected field remains visible, and all four actions remain available.
- At `40x10`, the existing `TERMINAL TOO SMALL` fallback still wins over the profile overlay.
- The selected text field highlights only its value/editor area, not the entire modal width.
- Driver options retain database-specific icon colors, but the selected Driver no longer uses a large high-saturation accent block.
- Cycle fields visibly communicate left/right adjustment with `‹ value ›`; toggles use `[ ] Off` / `[x] On`; drill-in fields use a trailing `›`.
- URL examples are absent unless the URL field is focused; focused URL help shows accepted schemes for the selected driver.
- An unfocused URL starts at the protocol and uses display-width-safe truncation instead of preserving the last cursor scroll offset.
- Connection test success uses success styling, warnings use amber, failures and validation use red, and busy operations use warning/action styling.
- Test feedback is concise: `Connection verified · PostgreSQL 17.6 · database`. Long server build metadata is not rendered in the main form.
- Editing a connection field after a successful test renders `Connection changed · Test again`.
- Buttons have a stable visual hierarchy and do not move when feedback changes.
- Passwords and URL passwords remain redacted in rendered output, debug output, and test failure messages.
- `cargo fmt --check`, `cargo check`, and all affected test suites pass.

## Non-Goals

- Do not redesign the Visible Objects picker or delete confirmation beyond shared typography/button improvements needed for consistency.
- Do not add tabs, collapsible cards, animation, a second modal, or new dependencies.
- Do not add a separate “Details” overlay for full server build metadata in this iteration. The runtime result remains available to the model; the primary form only shows the concise server summary.
- Do not change profile persistence, connection URL parsing, credential storage, or catalog discovery semantics.
- Do not change global application theme colors except adding one semantic success color.

---

### Task 1: Lock Down the Responsive Layout Contract

**Files:**
- Modify: `tests/ui_render.rs:2135-2554`
- Modify: `src/ui/profiles.rs:49-228`

**Step 1: Add a helper that locates the profile panel border**

Add a local test helper in `tests/ui_render.rs` that scans a `ratatui::buffer::Buffer` for the top border containing `NEW CONNECTION` or `EDIT CONNECTION` and returns its bounding `Rect`. Keep it test-only; do not expose production geometry.

```rust
fn profile_panel_rect(buffer: &ratatui::buffer::Buffer, title: &str) -> Rect {
    let area = buffer.area;
    let top = (area.y..area.bottom())
        .find(|&y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains(title)
        })
        .expect("profile title must be rendered");
    let left = (area.x..area.right())
        .find(|&x| buffer[(x, top)].symbol() == "╭")
        .expect("profile panel must have a top-left corner");
    let right = (area.x..area.right())
        .rev()
        .find(|&x| buffer[(x, top)].symbol() == "╮")
        .expect("profile panel must have a top-right corner");
    let bottom = (top.saturating_add(1)..area.bottom())
        .find(|&y| {
            buffer[(left, y)].symbol() == "╰"
                && buffer[(right, y)].symbol() == "╯"
        })
        .expect("profile panel must have a bottom border");

    Rect::new(
        left,
        top,
        right.saturating_sub(left).saturating_add(1),
        bottom.saturating_sub(top).saturating_add(1),
    )
}
```

Implement the scan using visible border symbols rather than hardcoded absolute coordinates. Support the rounded border emitted by Ratatui.

**Step 2: Write failing wide-layout tests**

Add `profile_form_is_centered_and_width_bounded_on_wide_terminals`:

```rust
#[test]
fn profile_form_is_centered_and_width_bounded_on_wide_terminals() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let (buffer, _) = render_buffer_with_icons(
        &app,
        160,
        40,
        IconSet::new(IconMode::Ascii),
    );

    let panel = profile_panel_rect(&buffer, "NEW CONNECTION");
    assert!(panel.width <= 106, "panel width was {}", panel.width);
    assert!(panel.x >= 20, "panel was not centered: {panel:?}");
    assert!(160 - panel.right() >= 20, "panel was not centered: {panel:?}");
    assert!(panel.x.abs_diff(160 - panel.right()) <= 1);
}
```

Add `profile_form_action_rows_stay_fixed_when_feedback_appears`, rendering the same form before and after assigning a two-line message and asserting `ProfileButton::SaveAndConnect` keeps the same `Rect`.

**Step 3: Run tests to verify they fail**

Run:

```bash
cargo test --test ui_render profile_form_is_centered_and_width_bounded_on_wide_terminals
cargo test --test ui_render profile_form_action_rows_stay_fixed_when_feedback_appears
```

Expected: the width assertion fails because the current panel max width is 96 rather than the new explicit contract, or the panel helper/action stability test exposes current geometry assumptions. Keep the tests as contract tests even if one happens to pass before implementation.

**Step 4: Introduce a render-only form geometry type**

In `src/ui/profiles.rs`, add private constants and a private layout struct near `render_form`:

```rust
const FORM_MAX_WIDTH: u16 = 106;
const FORM_MAX_HEIGHT: u16 = 34;
const FORM_COMPACT_HEIGHT: u16 = 26;
const FORM_LABEL_WIDTH: u16 = 20;

#[derive(Clone, Copy, Debug)]
struct FormLayout {
    panel: Rect,
    inner: Rect,
    header: Rect,
    body: Rect,
    feedback: Rect,
    actions: Rect,
    hint: Rect,
    show_sections: bool,
}
```

Add `form_layout(area, inner)` after `manager_panel`. The layout must reserve rows from bottom to top:

- 1 row hint
- 1 row actions
- 2 rows feedback
- remaining rows body
- 1 row header at the top

Use saturating arithmetic everywhere. `show_sections` is true only when the body has enough room for all visible fields plus section headings; do not derive it only from terminal width.

**Step 5: Refactor `render_form` to consume `FormLayout`**

Change `manager_panel(area, 96, 34)` to `manager_panel(area, FORM_MAX_WIDTH, FORM_MAX_HEIGHT)`. Route header, field viewport, feedback, actions, and hint through the named rectangles. Do not alter field text or styles yet.

The action and hint rows must always be based on the bottom of the same panel, regardless of feedback length.

**Step 6: Run the focused and existing compact tests**

Run:

```bash
cargo test --test ui_render profile_form_is_centered_and_width_bounded_on_wide_terminals
cargo test --test ui_render profile_form_action_rows_stay_fixed_when_feedback_appears
cargo test --test ui_render profile_form_remains_actionable_in_compact_layout
cargo test --test ui_render tiny_terminal_wins_over_profile_overlay
```

Expected: all pass.

**Step 7: Commit**

```bash
git add src/ui/profiles.rs tests/ui_render.rs
git commit -m "refactor(profiles): define responsive form layout"
```

---

### Task 2: Align Field Order With User Mental Models

**Files:**
- Modify: `src/model/profile_manager.rs:2050-2123`
- Modify: `tests/profile_draft.rs:650-730`
- Modify: `tests/profile_reducer.rs:454-483`

**Step 1: Write failing navigation-order tests**

Add table-driven tests that assert the visible field arrays have this order.

PostgreSQL:

```text
Kind
Name
Host
Port
Database
Schema
VisibleObjects
User
Password
PasswordStorage
SslMode
Environment
ReadOnly
Url
Test
Save
SaveAndConnect
Cancel
```

MySQL:

```text
Kind
Name
Host
Port
Database
VisibleObjects
User
Password
PasswordStorage
SslMode
Environment
ReadOnly
Url
Test
Save
SaveAndConnect
Cancel
```

SQLite file:

```text
Kind
Name
SqliteMemory
SqlitePath
VisibleObjects
ReadOnly
Url
Test
Save
SaveAndConnect
Cancel
```

SQLite memory is identical except `SqlitePath` is omitted.

Use exact array equality, not partial `contains` assertions.

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test profile_draft visible_fields_follow_product_sections
```

Expected: FAIL because authentication fields currently appear before database/scope fields.

**Step 3: Reorder the existing field constants**

Only reorder `POSTGRES_FIELDS`, `MYSQL_FIELDS`, `SQLITE_FILE_FIELDS`, and `SQLITE_MEMORY_FIELDS`. Do not add duplicate section rows to the model; sections are a render concern.

**Step 4: Update navigation expectations**

Update `form_navigation_skips_fields_hidden_by_driver_and_mode` and the visible-field navigation test so `ProfileFieldNext` follows the new exact visual order. Add one reducer assertion that moving from `VisibleObjects` goes to `User` for PostgreSQL and moving backward returns to `VisibleObjects`.

**Step 5: Run model and reducer tests**

Run:

```bash
cargo test --test profile_draft
cargo test --test profile_reducer form_navigation
```

Expected: all pass.

**Step 6: Commit**

```bash
git add src/model/profile_manager.rs tests/profile_draft.rs tests/profile_reducer.rs
git commit -m "refactor(profiles): group connection field navigation"
```

---

### Task 3: Add Sectioned Form Rows Without Changing Interaction State

**Files:**
- Modify: `src/ui/profiles.rs:49-228`
- Modify: `tests/ui_render.rs:2150-2409`

**Step 1: Write failing section tests**

Replace the broad `server_profile_form_shows_all_fields_and_never_reveals_passwords` label loop with two focused tests:

1. `wide_profile_form_groups_fields_into_product_sections`
2. `compact_profile_form_omits_section_rows_but_keeps_fields_actionable`

Wide output must contain, in order:

```text
CONNECTION
Driver
Name
Host
Database
AUTHENTICATION
User
Password
OPTIONS
SSL mode
Environment
CONNECTION URL
URL
```

Compact output at `80x24` must still contain the selected field and `Save & Connect`, but section headings may be omitted to preserve row capacity.

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test ui_render wide_profile_form_groups_fields_into_product_sections
cargo test --test ui_render compact_profile_form_omits_section_rows_but_keeps_fields_actionable
```

Expected: FAIL because no section headings exist and labels are all uppercase.

**Step 3: Add private section metadata**

In `src/ui/profiles.rs`, add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FormSection {
    Connection,
    Authentication,
    Options,
}

impl FormSection {
    const fn title(self) -> &'static str {
        match self {
            Self::Connection => "CONNECTION",
            Self::Authentication => "AUTHENTICATION",
            Self::Options => "OPTIONS",
        }
    }
}

const fn field_section(field: ProfileField) -> Option<FormSection> {
    match field {
        ProfileField::Kind
        | ProfileField::Name
        | ProfileField::Host
        | ProfileField::Port
        | ProfileField::Database
        | ProfileField::Schema
        | ProfileField::VisibleObjects
        | ProfileField::SqliteMemory
        | ProfileField::SqlitePath => Some(FormSection::Connection),
        ProfileField::User | ProfileField::Password | ProfileField::PasswordStorage => {
            Some(FormSection::Authentication)
        }
        ProfileField::SslMode | ProfileField::Environment | ProfileField::ReadOnly => {
            Some(FormSection::Options)
        }
        _ => None,
    }
}
```

Do not store section selection in `ProfileManagerState`.

**Step 4: Build render rows from visible fields**

Add a private `FormRow` enum:

```rust
enum FormRow {
    Section(FormSection),
    Field(ProfileField),
}
```

Add `form_rows(draft, show_sections) -> Vec<FormRow>`. It must:

- exclude button fields and URL;
- insert a heading only when the section changes;
- skip empty Authentication for SQLite;
- never insert blank spacer rows in compact mode;
- preserve the exact field order from `ProfileDraft::visible_fields()`.

Compute viewport selection against the row containing `manager.selected_field`, not against a separate field-only vector. Section rows are not selectable and receive no hit region.

**Step 5: Render restrained section headings**

Render headings with:

```rust
Style::new()
    .fg(theme.border)
    .bg(theme.surface)
    .add_modifier(Modifier::BOLD)
```

Use one leading space and no surrounding border. Do not add a horizontal rule; the outer panel is sufficient.

**Step 6: Convert labels and values to natural casing**

Update `field_label`, `kind_name`, `ssl_name`, `environment_name`, `password_storage_name`, and `toggle_value` output:

```text
Driver
URL
Name
Host
Port
User
Password
Database
Default schema
Visible objects
SSL mode
Environment
Read only
Password storage
Memory database
Path

PostgreSQL
MySQL
SQLite
Prefer
Development
Local encrypted
[x] On
[ ] Off
```

Keep section titles and operation names uppercase.

**Step 7: Run rendering and navigation tests**

Run:

```bash
cargo test --test ui_render wide_profile_form_groups_fields_into_product_sections
cargo test --test ui_render compact_profile_form_omits_section_rows_but_keeps_fields_actionable
cargo test --test ui_render mysql_and_sqlite_forms_only_show_relevant_fields
cargo test --test profile_reducer form_navigation
```

Expected: all pass.

**Step 8: Commit**

```bash
git add src/ui/profiles.rs tests/ui_render.rs
git commit -m "feat(profiles): group connection form fields"
```

---

### Task 4: Localize Focus and Add Control Affordances

**Files:**
- Modify: `src/ui/profiles.rs:428-608`
- Modify: `tests/ui_render.rs:2412-2474`
- Modify: `tests/mouse.rs:400-450`

**Step 1: Write failing buffer-style tests**

Add `profile_field_focus_is_limited_to_the_value_area`:

- focus `ProfileField::Name`;
- render with ASCII icons at `120x36`;
- find the `Name` row through `HitTarget::ProfileField(ProfileField::Name)`;
- assert the far-right cell of the row uses `theme.surface`, not `theme.selection`;
- assert at least one cell in the value area uses `theme.selection`;
- assert the indicator uses `theme.accent`.

Add `profile_cycle_and_drill_in_fields_expose_interaction_affordances` and assert output contains:

```text
‹ Prefer ›
‹ Development ›
1 database ›
```

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test ui_render profile_field_focus_is_limited_to_the_value_area
cargo test --test ui_render profile_cycle_and_drill_in_fields_expose_interaction_affordances
```

Expected: FAIL because active rows currently fill the complete row and cycle fields have no affordance.

**Step 3: Centralize field geometry**

Add:

```rust
#[derive(Clone, Copy, Debug)]
struct FieldLayout {
    row: Rect,
    label: Rect,
    value: Rect,
}

fn field_layout(area: Rect) -> FieldLayout {
    let label_width = area.width.min(FORM_LABEL_WIDTH);
    FieldLayout {
        row: area,
        label: Rect::new(area.x, area.y, label_width, 1),
        value: Rect::new(
            area.x.saturating_add(label_width),
            area.y,
            area.width.saturating_sub(label_width).min(64),
            1,
        ),
    }
}
```

The value width cap prevents focus backgrounds from stretching across the entire modal. Driver options may consume the full remaining width because they are a grouped control.

Use the same helper from `render_field`, `render_field_cursor`, and field scroll calculations. Remove duplicated literal `22` offsets.

**Step 4: Change active field rendering**

- Keep the whole row on `theme.surface`.
- Render `›` in accent for the active field.
- Render the active label in `theme.action` and inactive labels in `theme.muted`.
- Apply `theme.selection` only to the active value rectangle.
- Do not apply selection background to section headings or empty trailing cells.
- Keep the real terminal cursor for editable text fields.

**Step 5: Add value affordance formatting**

Add private helpers instead of embedding formatting inside `field_value`:

```rust
fn cycle_value_label(value: &str) -> String {
    format!("‹ {value} ›")
}

fn drill_in_value_label(value: &str) -> String {
    format!("{value} ›")
}
```

Apply cycle affordances to `SslMode`, `Environment`, and `PasswordStorage`. Apply drill-in affordance to `VisibleObjects`. Keep `Kind` as a segmented control and toggles as checkboxes.

**Step 6: Restyle Driver as a restrained segmented control**

Change selected Driver styling:

- icon keeps `driver_icon_color(kind)`;
- selected text uses `theme.accent` and bold;
- selected background uses `theme.selection`, not `theme.accent`;
- inactive text uses `theme.text` when the Driver row is focused, otherwise `theme.muted`;
- wrap only the selected label in `[ ... ]` or use a one-cell selection background, but keep hit targets separate and stable.

Pass an `active` boolean into `render_driver_options` so the control can distinguish “selected database” from “currently focused field.”

**Step 7: Preserve mouse and cursor behavior**

Keep the full field row clickable for usability, but ensure the cursor uses the value rectangle's x coordinate. Update mouse tests to assert:

- Driver options still have non-overlapping individual regions;
- Name row still maps to `ProfileFocusField(Name)`;
- Visible Objects still maps to the drill-in action;
- toggle rows still map to toggle actions.

**Step 8: Run tests**

Run:

```bash
cargo test --test ui_render profile_field_focus
cargo test --test ui_render profile_cycle_and_drill_in
cargo test --test ui_render driver_options
cargo test --test mouse profile
cargo test --test keymap profile_form
```

Expected: all pass.

**Step 9: Commit**

```bash
git add src/ui/profiles.rs tests/ui_render.rs tests/mouse.rs
git commit -m "feat(profiles): refine connection form focus states"
```

---

### Task 5: Replace Persistent URL Examples With Contextual Help

**Files:**
- Modify: `src/ui/profiles.rs:141-186, 552-608, 833-850`
- Modify: `tests/ui_render.rs:2150-2235, 2325-2346, 2394-2410`

**Step 1: Rewrite URL example tests as contextual-help tests**

Replace `profile_url_examples_follow_the_selected_driver` with `profile_url_help_only_appears_while_url_is_focused`.

For each Driver:

- render while `Name` is focused and assert no example URL is present;
- focus `ProfileField::Url` and assert exactly one protocol-help line is present;
- verify the help line matches the selected Driver.

Expected copy:

```text
PostgreSQL: Accepts postgres://, postgresql://, and jdbc:postgresql://
MySQL:      Accepts mysql:// and jdbc:mysql://
SQLite:     Accepts sqlite://, file:, and jdbc:sqlite:
```

Do not expose passwords in examples.

**Step 2: Add an unfocused URL-origin test**

Add `unfocused_profile_url_starts_at_the_protocol_after_cursor_editing`:

1. focus URL;
2. move the URL cursor to the end so horizontal scrolling occurs;
3. focus Name;
4. render;
5. assert output contains `postgresql://` or the active format's protocol prefix;
6. assert it does not begin from a middle username/host fragment.

Also retain `pending_url_redacts_an_embedded_password_before_commit` unchanged.

**Step 3: Run tests to verify they fail**

Run:

```bash
cargo test --test ui_render profile_url_help_only_appears_while_url_is_focused
cargo test --test ui_render unfocused_profile_url_starts_at_the_protocol_after_cursor_editing
```

Expected: FAIL because examples are always rendered and scroll offset follows the cursor even when URL is unfocused.

**Step 4: Reserve a stable URL help row**

The URL section always owns two rows:

1. editable URL row;
2. contextual help row, blank unless URL is focused or has a URL validation error.

This keeps the form stable when focus enters or leaves URL. Remove `example_count` and all dynamic fixed-height calculations from `render_form`.

**Step 5: Replace `examples` with `url_help`**

```rust
fn url_help(kind: DatabaseKind) -> &'static str {
    match kind {
        DatabaseKind::Postgres => {
            "Accepts postgres://, postgresql://, and jdbc:postgresql://"
        }
        DatabaseKind::MySql => "Accepts mysql:// and jdbc:mysql://",
        DatabaseKind::Sqlite => "Accepts sqlite://, file:, and jdbc:sqlite:",
    }
}
```

Render the help in `theme.muted`, aligned with the value area rather than the label column.

**Step 6: Separate focused editing from unfocused preview**

Change URL rendering:

- focused: render the full redacted URL and use cursor-based horizontal scrolling;
- unfocused: use horizontal offset `0` and truncate to the value rectangle;
- truncation must use display-cell width, not byte count;
- append `…` only when content exceeds the available width;
- never calculate truncation from the raw secret URL.

Add a private helper:

```rust
fn truncate_display(value: &str, width: u16) -> String {
    if value.cell_width() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }

    let target = width.saturating_sub(1);
    let mut output = String::new();
    let mut used = 0u16;
    for character in value.chars() {
        let character_width = character.to_string().as_str().cell_width();
        if used.saturating_add(character_width) > target {
            break;
        }
        output.push(character);
        used = used.saturating_add(character_width);
    }
    output.push('…');
    output
}
```

Use `safe_line(&draft.url_display())` as input.

**Step 7: Run URL, compact, and redaction tests**

Run:

```bash
cargo test --test ui_render profile_url
cargo test --test ui_render pending_url_redacts
cargo test --test ui_render compact_layout
cargo test --test profile_draft url
```

Expected: all pass.

**Step 8: Commit**

```bash
git add src/ui/profiles.rs tests/ui_render.rs
git commit -m "feat(profiles): show contextual connection URL help"
```

---

### Task 6: Introduce Typed Form Feedback

**Files:**
- Modify: `src/model/profile_manager.rs:1161-1217, 1524-1600`
- Modify: `src/app.rs:2295-2459, 5430-5580`
- Modify: `src/ui/theme.rs:16-66`
- Modify: `tests/profile_reducer.rs:675-786, 980-1045, 1390-1530`
- Modify: `tests/ui_render.rs:2476-2541`

**Step 1: Write failing typed-feedback reducer tests**

Add tests for these state transitions:

```rust
assert_eq!(feedback.kind, ProfileFeedbackKind::Error);   // validation failure
assert_eq!(feedback.kind, ProfileFeedbackKind::Success); // successful test
assert_eq!(feedback.kind, ProfileFeedbackKind::Warning); // test success + discovery warning
assert_eq!(feedback.kind, ProfileFeedbackKind::Error);   // failed test
```

For successful tests, assert concise fields instead of one preformatted build string:

```rust
assert_eq!(feedback.summary, "Connection verified");
assert_eq!(feedback.detail.as_deref(), Some("PostgreSQL 16.4 · lazydb"));
```

For discovery warning:

```rust
assert_eq!(feedback.summary, "Connection verified");
assert_eq!(feedback.detail.as_deref(), Some("Catalog unavailable: catalog permission denied"));
```

**Step 2: Run reducer tests to verify they fail**

Run:

```bash
cargo test --test profile_reducer test_rejects_invalid_drafts_and_tracks_matching_results
cargo test --test profile_reducer profile_test_discovery_failure_is_success_with_a_warning
```

Expected: compile failure because typed feedback does not exist.

**Step 3: Add feedback types**

In `src/model/profile_manager.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileFeedbackKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileFeedback {
    pub kind: ProfileFeedbackKind,
    pub summary: String,
    pub detail: Option<String>,
}

impl ProfileFeedback {
    pub fn new(
        kind: ProfileFeedbackKind,
        summary: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            kind,
            summary: summary.into(),
            detail,
        }
    }
}
```

Replace `ProfileManagerState.message: Option<String>` with `feedback: Option<ProfileFeedback>`.

**Step 4: Migrate feedback writers without compatibility shims**

Update every `manager.message = ...` assignment and every test reference. Do not keep both `message` and `feedback`.

Use these mappings:

- validation errors: `Error`, validation message as summary, no detail;
- password required: `Error`, `Password required`, no detail;
- test in progress: represented by `operation`, no feedback;
- test failed: `Error`, `Connection failed`, runtime message as detail;
- test succeeded: `Success`, `Connection verified`, concise server detail;
- test succeeded with catalog warning: `Warning`, `Connection verified`, warning detail;
- save completed but reconnect blocked by running query: `Warning`, `Profile saved`, actionable detail;
- unavailable credential store: `Warning`;
- informational stale/discarded operation messages: `Info`.

Use a helper for database display names so the reducer does not depend on UI-private `kind_name`.

**Step 5: Clear feedback on edits**

Replace `self.message = None` in insert, paste, delete, toggle, cycle, and Driver-selection methods with `self.feedback = None`. Keep cursor-only movement from clearing feedback.

This distinction is important: moving the cursor must not erase an error; changing a value may clear it.

**Step 6: Add a semantic success color**

Add `pub success: Color` to `Theme`. In `deep_space`, use a calm green that passes contrast against `surface`, for example:

```rust
success: Color::Rgb(92, 200, 150),
```

Update `src/ui/theme.rs` tests to assert success differs from warning and error.

**Step 7: Run model, reducer, and theme tests**

Run:

```bash
cargo test --test profile_reducer
cargo test --test profile_draft
cargo test ui::theme
```

Expected: all pass.

**Step 8: Commit**

```bash
git add src/model/profile_manager.rs src/app.rs src/ui/theme.rs tests/profile_reducer.rs
git commit -m "refactor(profiles): type connection form feedback"
```

---

### Task 7: Render Stable Semantic Status Feedback

**Files:**
- Modify: `src/ui/profiles.rs:75-88, 188, 610-631, 870-878`
- Modify: `tests/ui_render.rs:2476-2541`

**Step 1: Write failing semantic-color tests**

Split `profile_manager_renders_confirmation_busy_errors_and_warnings` into focused tests:

- `profile_form_renders_busy_state_and_disables_interaction`
- `profile_form_renders_success_feedback_with_success_color`
- `profile_form_renders_warning_feedback_with_warning_color`
- `profile_form_renders_error_feedback_with_error_color`
- `profile_form_marks_fresh_test_results_stale_after_connection_edits`

For color assertions, locate the first nonblank feedback glyph in the buffer and inspect its foreground color.

For stale state:

1. submit a valid profile test;
2. apply `ProfileTestSucceeded`;
3. edit Host;
4. render;
5. assert `Connection changed` and `Test again` appear.

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test ui_render profile_form_renders_success_feedback
cargo test --test ui_render profile_form_marks_fresh_test_results_stale
```

Expected: FAIL because all current messages are warning-colored and no stale summary is rendered.

**Step 3: Create a render-only status projection**

Add private types in `src/ui/profiles.rs`:

```rust
struct FormStatus<'a> {
    marker: &'static str,
    summary: &'a str,
    detail: Option<&'a str>,
    color: ratatui::style::Color,
}
```

Add `form_status(manager, theme) -> Option<FormStatus<'_>>` with this priority:

1. active operation;
2. explicit typed feedback;
3. stale `CatalogDiscoveryState`;
4. no status.

Markers:

```text
◌ busy/info
✓ success
! warning
× error
○ stale/not tested
```

ASCII icon mode does not need separate markers here because these symbols already exist elsewhere in the UI; if snapshot tests show width instability, use `*`, `+`, `!`, `x`, and `o` through `IconSet` instead.

**Step 4: Render a concise fixed-height status block**

Use `layout.feedback`, always clear/fill both rows with `theme.surface`, then render:

```text
✓ Connection verified · PostgreSQL 17.6 · moss_biz
```

If detail is a warning/error sentence that does not fit, wrap or truncate within the second reserved row. Never let feedback overlap actions.

Busy copy must use typographic ellipsis:

```text
Testing connection…
Saving profile…
Saving & connecting…
Connecting…
```

Do not render the full server compilation string in the form.

**Step 5: Simplify the header status**

Replace `POSTGRES PROFILE` / `BUSY // ...` with a calmer one-line header:

```text
PostgreSQL profile · Development
```

When the profile has a non-empty name, allow:

```text
lssc-uat · PostgreSQL · Development
```

Keep operation state exclusively in the feedback area so the header does not compete with it.

**Step 6: Run semantic feedback tests**

Run:

```bash
cargo test --test ui_render profile_form_renders
cargo test --test profile_reducer test_
cargo test --test profile_reducer running_queries_block
```

Expected: all pass.

**Step 7: Commit**

```bash
git add src/ui/profiles.rs tests/ui_render.rs
git commit -m "feat(profiles): render semantic connection feedback"
```

---

### Task 8: Establish Primary, Secondary, and Tertiary Actions

**Files:**
- Modify: `src/ui/profiles.rs:188-227, 633-687`
- Modify: `tests/ui_render.rs:2188-2197, 2394-2410, 2476-2541`
- Modify: `tests/mouse.rs:400-450`

**Step 1: Write failing action-hierarchy tests**

Add `profile_form_has_one_filled_primary_action`:

- render with no button selected;
- locate all four `ProfileButton` hit regions;
- assert only `SaveAndConnect` uses `theme.accent` background;
- assert Test and Save use `theme.surface_raised` with `theme.action` foreground;
- assert Cancel uses `theme.surface` or `theme.surface_raised` with `theme.muted` foreground.

Add `profile_form_busy_action_label_matches_operation` and assert `Testing…`, `Saving…`, or `Connecting…` appears while all action hit regions are disabled.

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test ui_render profile_form_has_one_filled_primary_action
cargo test --test ui_render profile_form_busy_action_label_matches_operation
```

Expected: FAIL because all inactive buttons currently share the same style and labels never change.

**Step 3: Replace tuple buttons with a render model**

Add:

```rust
#[derive(Clone, Copy)]
enum ButtonRole {
    Primary,
    Secondary,
    Tertiary,
}

struct FormButton<'a> {
    button: ProfileButton,
    label: &'a str,
    selected: bool,
    role: ButtonRole,
}
```

Pass `[FormButton; 4]` to `render_buttons` instead of tuples. Keep button order:

```text
Test | Save | Save & Connect | Cancel
```

**Step 4: Apply role-based styles**

- Primary: accent background even when not keyboard-selected; bold high-contrast text.
- Secondary: raised surface, action-colored text.
- Tertiary: surface background, muted text.
- Keyboard-selected state: add `Modifier::REVERSED` or a leading `›` without changing button width. Do not make every selected button look primary.
- Disabled: muted text on raised surface for all roles.

Keep the complete visible label inside each mouse hit region.

**Step 5: Add busy labels without moving the row**

Use labels that fit within the existing action allocation:

```text
Testing…
Saving…
Connecting…
```

Only the action corresponding to the operation changes text. If all inputs are disabled, preserve all four button slots so the row does not re-center or jump.

**Step 6: Add contextual footer hints**

Replace the fixed hint with `form_hint(selected_field, width)`:

- text fields: `Tab/Shift+Tab move  Ctrl+W delete word  Ctrl+A/E start/end`;
- cycle fields/Driver: `Left/Right change  Tab move  Ctrl+T test`;
- toggle/drill-in: `Enter/Space select  Tab move  Esc cancel`;
- action fields: `Ctrl+T test  Ctrl+S save  Ctrl+Enter save & connect  Esc cancel`;
- compact width: `^T Test  ^S Save  ^Enter Connect  Esc Close`.

Use consistent Title Case and shortcut casing in all variants.

**Step 7: Run rendering, keymap, and mouse tests**

Run:

```bash
cargo test --test ui_render profile_form
cargo test --test keymap profile_form
cargo test --test mouse profile
```

Expected: all pass.

**Step 8: Commit**

```bash
git add src/ui/profiles.rs tests/ui_render.rs tests/mouse.rs
git commit -m "feat(profiles): clarify connection form actions"
```

---

### Task 9: Verify All Drivers, Sizes, Secrets, and Interaction Modes

**Files:**
- Modify: `tests/ui_render.rs:2150-2554`
- Modify: `tests/profile_draft.rs`
- Modify: `tests/profile_reducer.rs`
- Modify: `tests/keymap.rs:1500-1643`
- Modify: `tests/mouse.rs`

**Step 1: Consolidate the final render matrix**

Add a table-driven integration test covering:

| Driver | Size | Expected |
|---|---:|---|
| PostgreSQL | `160x40` | sections, centered panel, auth fields, concise URL |
| PostgreSQL | `80x24` | selected field visible, actions visible, no permanent examples |
| MySQL | `120x36` | no Default schema, MySQL URL help when focused |
| SQLite file | `120x36` | Path and Memory database, no auth section |
| SQLite memory | `80x24` | no Path, actions remain visible |

Avoid giant golden strings. Assert landmarks, ordering, hit regions, and buffer styles.

**Step 2: Add security regression assertions**

For every relevant form state:

- rendered output excludes raw password;
- rendered URL excludes embedded password;
- debug output excludes secret;
- feedback detail excludes password-bearing connection URLs;
- URL help contains no realistic credentials.

**Step 3: Run the full affected test set**

Run:

```bash
cargo test --test ui_render
cargo test --test profile_draft
cargo test --test profile_reducer
cargo test --test keymap
cargo test --test mouse
```

Expected: all tests pass.

**Step 4: Run compile and formatting checks**

Run:

```bash
cargo fmt --check
cargo check
git diff --check
```

Expected: all commands succeed with no output from formatting/diff checks.

**Step 5: Perform manual terminal QA**

Run the application in a terminal that supports the configured icon mode and verify:

1. Open New Connection at approximately `160x40`.
2. Resize to `120x36`, `80x24`, and the project's minimum supported terminal size.
3. Tab through every PostgreSQL field and confirm focus never disappears behind the fixed status/action area.
4. Use mouse clicks on every Driver, text field, toggle, Visible Objects, and action.
5. Verify `Ctrl+T`, `Ctrl+S`, and `Ctrl+Enter` still trigger Test, Save, and Save & Connect.
6. Verify `Ctrl+W`, `Ctrl+U`, `Ctrl+A`, and `Ctrl+E` in Name, Host, User, Password, and URL.
7. Test a valid connection and confirm one-line success feedback with no layout jump.
8. Change Host after a successful test and confirm `Connection changed · Test again`.
9. Test invalid credentials and confirm red error feedback with an actionable detail.
10. Switch PostgreSQL → MySQL → SQLite and confirm sections, fields, Driver colors, and URL help update immediately.
11. Confirm no password is visible during any step.

Record any terminal-specific glyph-width issue before merging. If a glyph is unstable in Nerd Font mode, fix it through `IconSet`; do not add per-test workarounds to production code.

**Step 6: Commit final regression tests**

```bash
git add tests/ui_render.rs tests/profile_draft.rs tests/profile_reducer.rs tests/keymap.rs tests/mouse.rs
git commit -m "test(profiles): cover redesigned connection form"
```

---

## Final Verification

Run:

```bash
cargo fmt --check
cargo check
cargo test --test ui_render --test profile_draft --test profile_reducer --test keymap --test mouse
git diff --check
git status --short
```

Expected:

- formatting and compile checks succeed;
- all affected test suites pass;
- only intended connection-form files are staged or modified;
- unrelated existing changes such as `README.md` or other plan files remain untouched.

## Recommended Commit Sequence

1. `refactor(profiles): define responsive form layout`
2. `refactor(profiles): group connection field navigation`
3. `feat(profiles): group connection form fields`
4. `feat(profiles): refine connection form focus states`
5. `feat(profiles): show contextual connection URL help`
6. `refactor(profiles): type connection form feedback`
7. `feat(profiles): render semantic connection feedback`
8. `feat(profiles): clarify connection form actions`
9. `test(profiles): cover redesigned connection form`

## Rollback Boundaries

Each commit is intentionally reversible:

- Tasks 1–5 are render/layout changes and can be reverted without touching persistence or runtime commands.
- Task 6 is the only state-model migration and should be reverted together with Task 7 if semantic feedback causes regressions.
- Task 8 only changes action presentation, not shortcut mapping or reducer behavior.
- Task 9 contains tests and manual QA corrections only.

## Definition of Done

The redesign is complete when the acceptance criteria pass in automated tests and manual QA confirms that the form feels compact and stable at wide, medium, and compact terminal sizes. The implementation must preserve keyboard-first operation, mouse accessibility, secret redaction, Driver-specific behavior, and existing connection lifecycle semantics.
