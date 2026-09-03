# Pending Transactions Panel Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Redesign the pending-transactions confirmation panel to match the visual hierarchy of the connection-group editor while preserving all existing commit, rollback, abandon, cancel, and keyboard behavior.

**Architecture:** Keep `Overlay::TransactionExitConfirm`, `TransactionExitChoice`, key mappings, and the transaction reducer unchanged. Extract the transaction-exit rendering branch from `render_overlay` into focused UI helpers that render a framed panel, a compact transaction summary, state-aware actions, and a bottom-aligned keyboard hint; derive every displayed state from the existing `App` and overlay data. Add rendering tests around the Ratatui `TestBackend` so layout and semantic styling regressions are caught without introducing snapshots or new dependencies.

**Tech Stack:** Rust 2024, Ratatui 0.30.2, Crossterm 0.29, existing `App` reducer/overlay architecture, Rust integration tests using `TestBackend`.

---

## Product Decisions

1. The panel border owns the title. Do not render `PENDING TRANSACTIONS` or `TRANSACTION` again inside the panel body.
2. Continue using `PENDING TRANSACTIONS` for `DeferredIntent::Quit` and `TRANSACTION` for all other deferred intents. This preserves the current distinction between quitting the application and resolving one transaction before another action.
3. Use `TRANSACTION SUMMARY` as the internal section label, matching the connection-group editor's `GROUP DETAILS` hierarchy.
4. Render one row per pending console. Prefix the prompt currently being resolved with `›`; queued prompts use a blank prefix.
5. Render transaction state labels in user-facing uppercase text rather than exposing Rust `Debug` output. Use `ACTIVE`, `ABORTED`, `STARTING`, `COMMITTING`, `ROLLING BACK`, `OUTCOME UNKNOWN`, and `IDLE`; preserve the existing `GONE` fallback when a deferred prompt references a tab that no longer exists.
6. Use existing theme colors only: `warning` for active work, `error` for aborted or unknown outcomes, `action` for transitional states, and `muted` for idle or unavailable information.
7. Keep Rollback as the default selected action. The selected Commit or Rollback action uses the same accent-filled treatment as the connection-group editor's primary action.
8. Keep Cancel outside the Commit/Rollback selection cycle because `ToggleTransactionExitChoice` currently toggles only Commit and Rollback in the normal state. Display Cancel as a muted secondary action and keep `Esc`/`n` as its input paths.
9. When the current transaction is aborted, render Commit as disabled and muted. Do not change reducer behavior; the existing reducer already rejects an invalid commit and restores Rollback.
10. When a query is running, replace the normal action row with a warning message. Preserve the current instruction to wait or use `Ctrl-C`; do not add new key handling in this change.
11. When the outcome is unknown, render a dedicated warning and only the existing Abandon/Cancel choices. Do not show Commit or Rollback.
12. Keep the popup compact: target width `68`, use at most the available terminal width through `centered`, and calculate height from the transaction count with a minimum of `9`. Let `centered` apply the terminal-height ceiling instead of imposing a lower fixed cap that would hide transactions on a tall terminal.
13. Keep the bottom keyboard hint centered on the final inner row, matching the connection-group overlay.
14. Mouse support is out of scope. No new `HitTarget` variants should be introduced because transaction overlays currently have no mouse contract.
15. Do not change `src/model/transaction.rs`, `src/input/keymap.rs`, `src/action.rs`, or transaction resolution code in `src/app.rs` unless a test proves an existing behavior is broken independently of this visual change.

## Acceptance Criteria

- The quit panel renders `PENDING TRANSACTIONS` exactly once.
- A non-quit transaction-exit panel renders `TRANSACTION` only as the border title; the body starts with `TRANSACTION SUMMARY` and does not repeat a standalone title line.
- The panel contains `TRANSACTION SUMMARY`, followed by all pending consoles and their states whenever the terminal can display them; terminal height remains the only clipping constraint.
- The current prompt row starts with `›`; deferred rows remain visually secondary.
- The current console name is sanitized and truncated to the row's available terminal-cell width instead of overflowing into the state column.
- Rollback is selected by default and has an accent background with bold text.
- Toggling the existing choice to Commit moves the selected styling to Commit without changing any keymap or reducer code.
- An aborted transaction renders Commit muted and Rollback selected.
- A running query shows a non-actionable warning and does not show the normal Commit/Rollback action row.
- An unknown outcome shows Abandon local state and Cancel, but no Commit or Rollback actions.
- The footer text accurately describes the controls available in each state.
- The panel remains bordered, readable, and free of panic at `56 x 16`, the repository's established minimum terminal size.
- Existing transaction flow and keymap tests continue to pass unchanged.
- Formatting, the focused UI tests, the full test suite, `cargo check`, and Clippy pass.

## Non-Goals

- Changing the default rollback policy.
- Changing transaction commit, rollback, cancellation, or deferred-intent sequencing.
- Adding confirmation steps or changing direct `c`, `r`, `a`, `n`, `Enter`, `Tab`, arrow, or `Esc` shortcuts.
- Redesigning `RelationTransactionConfirm`, `ManualCancelConfirm`, or `ClearTransactionOutcome` in the same patch.
- Introducing a generic modal framework or general-purpose button component.
- Adding mouse interaction to transaction overlays.
- Adding a new theme color, icon, crate, or persisted setting.

---

### Task 1: Lock Down the Normal Panel Contract

**Files:**
- Modify: `tests/ui_render.rs:2785-2804`

**Step 1: Extend the existing multi-transaction test with structural assertions**

Rename `quit_panel_lists_all_pending_transactions` to `quit_panel_uses_compact_transaction_summary_layout` and keep its existing fixture setup. Replace the final assertions with checks for the new stable UI contract:

```rust
assert_eq!(output.matches("PENDING TRANSACTIONS").count(), 1, "{output}");
assert!(output.contains("TRANSACTION SUMMARY"), "{output}");
assert!(output.contains("Active") || output.contains("ACTIVE"), "{output}");
assert!(output.contains("Aborted") || output.contains("ABORTED"), "{output}");
assert!(output.contains("Commit"), "{output}");
assert!(output.contains("Rollback"), "{output}");
assert!(output.contains("Esc cancel"), "{output}");
assert!(!output.contains("Rollback is the default"), "{output}");
```

The temporary mixed-case alternatives allow this first test to isolate layout changes from the state-label change in Task 3. Task 3 will tighten them to uppercase-only assertions.

**Step 2: Add a selected-action style test**

Add a test immediately after the layout test. Reuse `render_buffer_with_icons` and `find_text_cell`, which already exist in this file:

```rust
#[test]
fn quit_panel_highlights_rollback_as_the_default_action() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    assert!(app.update(Action::Quit).is_empty());

    let (buffer, _) =
        render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let (rollback_x, rollback_y) = find_text_cell(&buffer, "Rollback").expect("rollback action");
    let (commit_x, commit_y) = find_text_cell(&buffer, "Commit").expect("commit action");

    assert_eq!(buffer[(rollback_x, rollback_y)].bg, Color::Rgb(99, 230, 216));
    assert_eq!(buffer[(rollback_x, rollback_y)].fg, Color::Rgb(7, 11, 18));
    assert!(
        buffer[(rollback_x, rollback_y)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert_ne!(buffer[(commit_x, commit_y)].bg, buffer[(rollback_x, rollback_y)].bg);
}
```

Use the existing deep-space theme constants already asserted elsewhere in `tests/ui_render.rs`; do not expose `Theme` publicly just for tests.

**Step 3: Add a non-quit title-ownership test**

Create a manual active transaction and open the existing transaction-control flow:

```rust
#[test]
fn transaction_panel_keeps_the_title_out_of_the_body() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.update(Action::OpenTransactionControl);

    let output = render(&app, 100, 30);
    let title_line = output
        .lines()
        .find(|line| line.contains(" TRANSACTION "))
        .expect("transaction border title");

    assert!(title_line.contains('─'), "{output}");
    assert!(output.contains("TRANSACTION SUMMARY"), "{output}");
    assert_eq!(
        output.lines().filter(|line| line.trim() == "TRANSACTION").count(),
        0,
        "{output}"
    );
}
```

This avoids counting `TRANSACTION` inside the intentional `TRANSACTION SUMMARY` section label.

**Step 4: Run the focused tests and confirm failure**

Run:

```bash
cargo test --test ui_render quit_panel_ -- --nocapture
```

Expected: the tests fail because the current body repeats the title, lacks `TRANSACTION SUMMARY`, retains the default-policy sentence, and represents selection only with brackets rather than an accent background.

**Step 5: Commit the failing tests**

```bash
git add tests/ui_render.rs
git commit -m "test(ui): define pending transaction panel layout"
```

---

### Task 2: Extract and Implement the Compact Normal Layout

**Files:**
- Modify: `src/ui/mod.rs:2999-3087`
- Modify: `src/ui/mod.rs` near `render_profile_group_actions` at `3568`

**Step 1: Replace the inline overlay branch with delegation**

Reduce the existing `Overlay::TransactionExitConfirm` match arm to:

```rust
Overlay::TransactionExitConfirm { prompt, choice } => {
    render_transaction_exit_overlay(frame, area, app, prompt, *choice, theme);
}
```

Keep this branch in the same position so overlay ordering and `overlay_key` remain unchanged.

**Step 2: Add the focused renderer**

Place `render_transaction_exit_overlay` immediately before `render_profile_group_overlay`, keeping overlay-specific renderers together. Its initial implementation should handle the normal non-running, known-outcome path:

```rust
fn render_transaction_exit_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    prompt: &crate::model::transaction::DeferredTransactionPrompt,
    choice: crate::model::transaction::TransactionExitChoice,
    theme: Theme,
) {
    use crate::model::transaction::{DeferredIntent, TransactionExitChoice, TransactionState};

    let pending = std::iter::once(prompt.console_id)
        .chain(
            app.deferred_transaction_prompts()
                .filter(|queued| queued.intent == prompt.intent)
                .map(|queued| queued.console_id),
        )
        .collect::<Vec<_>>();
    let popup = centered(
        area,
        68,
        (pending.len() as u16).saturating_add(7).max(9),
    );
    frame.render_widget(Clear, popup);

    let title = if prompt.intent == DeferredIntent::Quit {
        " PENDING TRANSACTIONS "
    } else {
        " TRANSACTION "
    };
    let block = panel_block(title, true, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    frame.render_widget(
        Paragraph::new("TRANSACTION SUMMARY").style(
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    let row_start = inner.y.saturating_add(2);
    for (index, id) in pending.iter().enumerate() {
        let y = row_start.saturating_add(index as u16);
        if y >= inner.bottom().saturating_sub(2) {
            break;
        }
        let tab = app.tabs.iter().find(|tab| tab.id() == *id);
        let state = tab
            .and_then(|tab| tab.as_console())
            .map(|console| console.transaction_state);
        render_transaction_summary_row(
            frame,
            Rect::new(inner.x, y, inner.width, 1),
            index == 0,
            tab.map_or("unknown", |tab| tab.title()),
            state,
            theme,
        );
    }

    let current = app.tabs.iter().find(|tab| tab.id() == prompt.console_id);
    let commit_enabled = !current.and_then(|tab| tab.as_console()).is_some_and(|console| {
        console.transaction_state == TransactionState::Aborted
    });
    render_transaction_exit_actions(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(2), inner.width, 1),
        choice,
        commit_enabled,
        theme,
    );
    frame.render_widget(
        Paragraph::new("Tab/←/→ select   Enter confirm   Esc cancel")
            .style(Style::new().fg(theme.muted).bg(theme.surface))
            .alignment(Alignment::Center),
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
    );
}
```

Do not leave the imported `TransactionExitChoice` unused: Task 4 extends this function with special-state branches. If implementing task-by-task creates a temporary warning, omit that item from the local import until Task 4.

**Step 3: Add the summary-row renderer**

Add a small renderer below `render_transaction_exit_overlay`. Reserve enough space for the longest status label, sanitize the title, and truncate by terminal-cell width:

```rust
fn render_transaction_summary_row(
    frame: &mut Frame<'_>,
    area: Rect,
    current: bool,
    title: &str,
    state: Option<crate::model::transaction::TransactionState>,
    theme: Theme,
) {
    let (state_label, state_color) = transaction_state_display(state, theme);
    let marker = if current { "› " } else { "  " };
    let state_width = state_label.cell_width();
    let title_width = area
        .width
        .saturating_sub(marker.cell_width())
        .saturating_sub(state_width)
        .saturating_sub(2);
    let title = truncate_to_cell_width(&sanitize_terminal_text(title), title_width);
    let padding = " ".repeat(usize::from(title_width.saturating_sub(title.cell_width())) + 2);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                marker,
                Style::new()
                    .fg(if current { theme.action } else { theme.muted })
                    .bg(theme.surface)
                    .add_modifier(if current {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                title,
                Style::new()
                    .fg(if current { theme.text } else { theme.muted })
                    .bg(theme.surface),
            ),
            Span::styled(padding, Style::new().bg(theme.surface)),
            Span::styled(
                state_label,
                Style::new()
                    .fg(state_color)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        area,
    );
}
```

If `sanitize_terminal_text` returns a borrowed/owned type that does not satisfy the call above, bind it first and pass `sanitized.as_str()` to `truncate_to_cell_width`. Do not bypass sanitization.

**Step 4: Add the action-row renderer**

Add `render_transaction_exit_actions` next to `render_profile_group_actions`. It should compute cell widths and center all three actions as one group:

```rust
fn render_transaction_exit_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    choice: crate::model::transaction::TransactionExitChoice,
    commit_enabled: bool,
    theme: Theme,
) {
    use crate::model::transaction::TransactionExitChoice;

    let commit = "[ Commit ]";
    let rollback = "[ Rollback ]";
    let cancel = "Cancel";
    let gap = 2;
    let total_width = commit
        .cell_width()
        .saturating_add(rollback.cell_width())
        .saturating_add(cancel.cell_width())
        .saturating_add(gap * 2);
    let mut x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);

    let actions = [
        (commit, TransactionExitChoice::Commit, commit_enabled),
        (rollback, TransactionExitChoice::Rollback, true),
    ];
    for (label, action, enabled) in actions {
        let width = label.cell_width();
        let selected = enabled && choice == action;
        let style = if selected {
            Style::new()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else if enabled {
            Style::new().fg(theme.text).bg(theme.surface)
        } else {
            Style::new().fg(theme.muted).bg(theme.surface)
        };
        frame.render_widget(
            Paragraph::new(label).style(style),
            Rect::new(x, area.y, width, 1),
        );
        x = x.saturating_add(width).saturating_add(gap);
    }
    frame.render_widget(
        Paragraph::new(cancel).style(
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(x, area.y, cancel.cell_width(), 1),
    );
}
```

Use `u16` values consistently if inference around `gap` requires an annotation.

**Step 5: Add the initial display mapping**

Add `transaction_state_display` below the row renderer. Task 3 will verify every mapping:

```rust
fn transaction_state_display(
    state: Option<crate::model::transaction::TransactionState>,
    theme: Theme,
) -> (&'static str, Color) {
    use crate::model::transaction::TransactionState;

    match state {
        Some(TransactionState::Active) => ("ACTIVE", theme.warning),
        Some(TransactionState::Aborted) => ("ABORTED", theme.error),
        Some(TransactionState::Starting) => ("STARTING", theme.action),
        Some(TransactionState::Committing) => ("COMMITTING", theme.action),
        Some(TransactionState::RollingBack) => ("ROLLING BACK", theme.action),
        Some(TransactionState::OutcomeUnknown) => ("OUTCOME UNKNOWN", theme.error),
        Some(TransactionState::Idle) => ("IDLE", theme.muted),
        None => ("GONE", theme.muted),
    }
}
```

`Color` is already imported by `src/ui/mod.rs`; if not, use `ratatui::style::Color` in the return type rather than adding a second import block.

**Step 6: Run the focused tests**

Run:

```bash
cargo test --test ui_render quit_panel_ -- --nocapture
```

Expected: PASS. The title appears only in the border, the section label is present, the default-policy prose is gone, and Rollback uses the accent selection style.

**Step 7: Format and compile-check the implementation**

Run:

```bash
cargo fmt --check
cargo check
```

Expected: both commands exit successfully with no warning caused by the new helpers.

**Step 8: Commit the normal layout**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "refactor(ui): redesign pending transaction panel"
```

---

### Task 3: Verify State Labels, Current Row, and Disabled Commit

**Files:**
- Modify: `tests/ui_render.rs` next to the pending-transaction tests
- Modify only if tests expose a defect: `src/ui/mod.rs` transaction summary helpers

**Step 1: Tighten the multi-transaction label assertions**

Replace the temporary mixed-case checks from Task 1 with:

```rust
assert!(output.contains("ACTIVE"), "{output}");
assert!(output.contains("ABORTED"), "{output}");
assert!(!output.contains("Active"), "{output}");
assert!(!output.contains("Aborted"), "{output}");
```

**Step 2: Add a current-row and state-color test**

Use the same two-console setup as the layout test and inspect the rendered buffer:

```rust
#[test]
fn quit_panel_marks_the_current_transaction_and_colors_states() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.update(Action::NewConsole);
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Aborted;
    assert!(app.update(Action::Quit).is_empty());

    let (buffer, _) =
        render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let (active_x, active_y) = find_text_cell(&buffer, "ACTIVE").expect("active state");
    let (aborted_x, aborted_y) = find_text_cell(&buffer, "ABORTED").expect("aborted state");
    let marker_x = (0..active_x)
        .rev()
        .find(|x| buffer[(*x, active_y)].symbol() == "›")
        .expect("current transaction marker");

    assert_eq!(buffer[(active_x, active_y)].fg, Color::Rgb(244, 184, 96));
    assert_eq!(buffer[(aborted_x, aborted_y)].fg, Color::Rgb(255, 107, 122));
    assert_eq!(buffer[(marker_x, active_y)].fg, Color::Rgb(101, 167, 255));
}
```

Confirm which queued row is first from the rendered output before finalizing the marker assertion. The renderer intentionally treats `pending[0]`, which is `prompt.console_id`, as current; do not infer currentness from `app.active_tab`.

**Step 3: Add an aborted-transaction action test**

```rust
#[test]
fn quit_panel_disables_commit_for_an_aborted_transaction() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Aborted;
    assert!(app.update(Action::Quit).is_empty());

    let (buffer, _) =
        render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let (commit_x, commit_y) = find_text_cell(&buffer, "Commit").expect("commit action");
    let (rollback_x, rollback_y) = find_text_cell(&buffer, "Rollback").expect("rollback action");

    assert_eq!(buffer[(commit_x, commit_y)].fg, Color::Rgb(105, 126, 146));
    assert_ne!(buffer[(commit_x, commit_y)].bg, Color::Rgb(99, 230, 216));
    assert_eq!(buffer[(rollback_x, rollback_y)].bg, Color::Rgb(99, 230, 216));
}
```

**Step 4: Add a choice-toggle rendering test**

```rust
#[test]
fn quit_panel_moves_selection_style_to_commit() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    assert!(app.update(Action::Quit).is_empty());
    app.update(Action::ToggleTransactionExitChoice);

    let (buffer, _) =
        render_buffer_with_icons(&app, 100, 30, IconSet::new(IconMode::Ascii));
    let (commit_x, commit_y) = find_text_cell(&buffer, "Commit").expect("commit action");
    let (rollback_x, rollback_y) = find_text_cell(&buffer, "Rollback").expect("rollback action");

    assert_eq!(buffer[(commit_x, commit_y)].bg, Color::Rgb(99, 230, 216));
    assert_ne!(buffer[(rollback_x, rollback_y)].bg, Color::Rgb(99, 230, 216));
}
```

This verifies presentation only while exercising the existing reducer action. Do not add a second selection state to the UI layer.

**Step 5: Run the focused tests**

Run:

```bash
cargo test --test ui_render quit_panel_ -- --nocapture
```

Expected: PASS. If the state marker test fails because the fixture's deferred queue order differs, inspect `output` and correct only the test setup; do not change application queue semantics for visual ordering.

**Step 6: Commit state presentation coverage**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "test(ui): cover transaction panel states"
```

---

### Task 4: Render Running and Unknown Outcomes as Dedicated States

**Files:**
- Modify: `tests/ui_render.rs` next to the pending-transaction tests
- Modify: `src/ui/mod.rs` in `render_transaction_exit_overlay`

**Step 1: Add a failing running-query test**

```rust
#[test]
fn quit_panel_replaces_transaction_actions_while_query_is_running() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.active_console_mut().query_status = QueryStatus::Running;
    assert!(app.update(Action::Quit).is_empty());

    let output = render(&app, 100, 30);

    assert!(output.contains("QUERY IN PROGRESS"), "{output}");
    assert!(output.contains("wait or Ctrl-C to cancel"), "{output}");
    assert!(!output.contains("[ Commit ]"), "{output}");
    assert!(!output.contains("[ Rollback ]"), "{output}");
    assert!(output.contains("Esc return"), "{output}");
}
```

**Step 2: Add a failing unknown-outcome test**

```rust
#[test]
fn quit_panel_isolates_unknown_outcome_actions() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::OutcomeUnknown;
    assert!(app.update(Action::Quit).is_empty());

    let output = render(&app, 100, 30);

    assert!(output.contains("OUTCOME UNKNOWN"), "{output}");
    assert!(output.contains("Abandon local state"), "{output}");
    assert!(!output.contains("[ Commit ]"), "{output}");
    assert!(!output.contains("[ Rollback ]"), "{output}");
    assert!(output.contains("A abandon"), "{output}");
    assert!(output.contains("Esc cancel"), "{output}");
}
```

If `Action::Quit` deliberately does not enqueue `OutcomeUnknown`, construct the existing public `Overlay::TransactionExitConfirm` directly with `DeferredTransactionPrompt` and `TransactionExitChoice::Abandon`. Do not alter production flow merely to simplify the test.

**Step 3: Run both tests and confirm failure**

Run:

```bash
cargo test --test ui_render quit_panel_replaces_transaction_actions_while_query_is_running -- --nocapture
cargo test --test ui_render quit_panel_isolates_unknown_outcome_actions -- --nocapture
```

Expected: FAIL because the extracted normal renderer currently always draws Commit/Rollback actions.

**Step 4: Branch the action area by current transaction state**

In `render_transaction_exit_overlay`, derive these booleans once from the current prompt tab:

```rust
let current_console = app
    .tabs
    .iter()
    .find(|tab| tab.id() == prompt.console_id)
    .and_then(|tab| tab.as_console());
let running = current_console.is_some_and(|console| console.query_status == QueryStatus::Running);
let outcome_unknown = current_console.is_some_and(|console| {
    console.transaction_state == TransactionState::OutcomeUnknown
});
let commit_enabled = !current_console.is_some_and(|console| {
    console.transaction_state == TransactionState::Aborted
});
```

Replace the unconditional normal action/footer rendering with:

```rust
let action_area = Rect::new(
    inner.x,
    inner.bottom().saturating_sub(2),
    inner.width,
    1,
);
let footer_area = Rect::new(
    inner.x,
    inner.bottom().saturating_sub(1),
    inner.width,
    1,
);

if running {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "QUERY IN PROGRESS",
                Style::new()
                    .fg(theme.warning)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  wait or Ctrl-C to cancel",
                Style::new().fg(theme.muted).bg(theme.surface),
            ),
        ]))
        .alignment(Alignment::Center),
        action_area,
    );
    frame.render_widget(
        Paragraph::new("Esc return")
            .style(Style::new().fg(theme.muted).bg(theme.surface))
            .alignment(Alignment::Center),
        footer_area,
    );
} else if outcome_unknown {
    render_unknown_transaction_actions(frame, action_area, choice, theme);
    frame.render_widget(
        Paragraph::new("A abandon   Esc cancel")
            .style(Style::new().fg(theme.muted).bg(theme.surface))
            .alignment(Alignment::Center),
        footer_area,
    );
} else {
    render_transaction_exit_actions(
        frame,
        action_area,
        choice,
        commit_enabled,
        theme,
    );
    frame.render_widget(
        Paragraph::new("Tab/←/→ select   Enter confirm   Esc cancel")
            .style(Style::new().fg(theme.muted).bg(theme.surface))
            .alignment(Alignment::Center),
        footer_area,
    );
}
```

Keep the summary row visible in all branches so the affected console and `OUTCOME UNKNOWN` remain identifiable.

**Step 5: Add the unknown-outcome action renderer**

Place it beside `render_transaction_exit_actions`:

```rust
fn render_unknown_transaction_actions(
    frame: &mut Frame<'_>,
    area: Rect,
    choice: crate::model::transaction::TransactionExitChoice,
    theme: Theme,
) {
    use crate::model::transaction::TransactionExitChoice;

    let abandon = "[ Abandon local state ]";
    let cancel = "Cancel";
    let gap = 2;
    let total_width = abandon
        .cell_width()
        .saturating_add(cancel.cell_width())
        .saturating_add(gap);
    let x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);
    frame.render_widget(
        Paragraph::new(abandon).style(if choice == TransactionExitChoice::Abandon {
            Style::new()
                .fg(theme.background)
                .bg(theme.error)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.text).bg(theme.surface)
        }),
        Rect::new(x, area.y, abandon.cell_width(), 1),
    );
    frame.render_widget(
        Paragraph::new(cancel).style(
            Style::new()
                .fg(theme.muted)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            x.saturating_add(abandon.cell_width()).saturating_add(gap),
            area.y,
            cancel.cell_width(),
            1,
        ),
    );
}
```

Use `theme.error` for the selected destructive local-state action so it cannot be mistaken for a routine commit/rollback choice.

**Step 6: Run all pending-panel tests**

Run:

```bash
cargo test --test ui_render quit_panel_ -- --nocapture
```

Expected: PASS for normal, aborted, running, and unknown-outcome states.

**Step 7: Run existing transaction behavior tests**

Run:

```bash
cargo test --test keymap transaction
cargo test --test app_flow transaction
cargo test --test transaction_reducer
```

Expected: PASS with no changes to key mappings or reducer behavior. If a name filter executes zero tests, rerun the corresponding whole integration-test target and record that result instead.

**Step 8: Commit special-state rendering**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "fix(ui): clarify pending transaction states"
```

---

### Task 5: Verify Responsive Layout and Text Safety

**Files:**
- Modify: `tests/ui_render.rs` next to the pending-transaction tests
- Modify only if tests expose a defect: `src/ui/mod.rs` transaction summary helpers

**Step 1: Add a minimum-size rendering test**

Use a long console title so the test exercises terminal-cell truncation rather than only a short fixture name:

```rust
#[test]
fn quit_panel_remains_readable_at_minimum_terminal_size() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    app.active_console_mut().name =
        "a-very-long-console-name-that-must-not-overwrite-the-state".into();
    assert!(app.update(Action::Quit).is_empty());

    let output = render(&app, 56, 16);

    assert_eq!(output.matches("PENDING TRANSACTIONS").count(), 1, "{output}");
    assert!(output.contains("TRANSACTION SUMMARY"), "{output}");
    assert!(output.contains("ACTIVE"), "{output}");
    assert!(output.contains("Rollback"), "{output}");
    assert!(output.contains("Esc cancel"), "{output}");
}
```

`ConsoleTab::name` is public in `src/model/tab.rs:173`, and `WorkspaceTab::title()` returns it for SQL tabs. Set `name` directly only in this fixture; do not alter the associated `ConsoleRecord`, because the overlay reads the open tab title.

**Step 2: Add a border-integrity assertion**

Render through `render_buffer_with_icons`, locate `PENDING TRANSACTIONS`, then scan its row for the opening rounded border and its column for the closing border. Follow the existing completion-popup border assertions around `tests/ui_render.rs:2272-2296`; do not add a screenshot dependency.

At minimum, assert that the rendered output contains all four rounded corners:

```rust
assert!(output.contains('╭'), "{output}");
assert!(output.contains('╮'), "{output}");
assert!(output.contains('╰'), "{output}");
assert!(output.contains('╯'), "{output}");
```

Prefer coordinate-specific assertions if unrelated panels also use rounded borders in this fixture.

**Step 3: Run the minimum-size test and fix only observed overflow**

Run:

```bash
cargo test --test ui_render quit_panel_remains_readable_at_minimum_terminal_size -- --nocapture
```

Expected: PASS. If the full normal footer does not fit at width 56, use a width-aware footer rather than reducing global popup margins:

```rust
let footer = if inner.width >= 52 {
    "Tab/←/→ select   Enter confirm   Esc cancel"
} else {
    "Tab select   Enter confirm   Esc cancel"
};
```

Keep the ASCII words `Enter` and `Esc` in both variants for accessibility and testability.

**Step 4: Run all UI render tests**

Run:

```bash
cargo test --test ui_render
```

Expected: PASS. Pay special attention to unrelated overlay tests because `render_overlay` was structurally edited.

**Step 5: Commit responsive coverage**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "test(ui): verify compact transaction panel"
```

---

### Task 6: Final Regression and Quality Gate

**Files:**
- Verify only: `src/ui/mod.rs`
- Verify only: `tests/ui_render.rs`

**Step 1: Review the final diff for scope**

Run:

```bash
git diff -- src/ui/mod.rs tests/ui_render.rs
```

Expected: only the transaction-exit render branch, its focused helpers, and pending-panel tests changed. Confirm there are no changes to transaction state transitions, keymaps, persistence, or database backends.

**Step 2: Format the workspace**

Run:

```bash
cargo fmt --check
```

Expected: PASS. If it fails, run `cargo fmt`, inspect the resulting diff, then rerun `cargo fmt --check`.

**Step 3: Run focused integration tests**

Run:

```bash
cargo test --test ui_render
cargo test --test keymap
cargo test --test app_flow
cargo test --test transaction_reducer
```

Expected: all targets PASS.

**Step 4: Run the full test suite**

Run:

```bash
cargo test --all-targets
```

Expected: PASS with no ignored failure attributable to the panel change.

**Step 5: Run compiler and lint checks**

Run:

```bash
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
```

Expected: both commands PASS without unused imports, needless allocation, manual saturation, or style warnings in the new render helpers.

**Step 6: Perform a manual visual check**

Run LazyDB using the project's normal development command and open a manual transaction, then trigger quit. Verify these cases in a real terminal:

1. One active transaction: title appears once and Rollback is selected.
2. Two pending transactions: both rows align and the current row has `›`.
3. Aborted transaction: Commit looks disabled.
4. Running query: normal actions disappear and the warning is visible.
5. Unknown outcome, if reproducible safely: only Abandon and Cancel appear.
6. Resize to approximately `56 x 16`: borders, state, action, and footer remain legible.

Expected: the panel visually matches the connection-group overlay's border, section-label, action-row, and footer hierarchy without altering keyboard behavior.

**Step 7: Create a final cleanup commit only if verification changed files**

If formatting or a verified responsive fix changed files:

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "chore(ui): polish transaction panel layout"
```

Do not create an empty commit when the working tree is already clean.

---

## Implementation Notes

- `src/ui/mod.rs:2999-3087` is the only production behavior surface intended for replacement. The new helpers should remain in the same module so they can reuse `panel_block`, `centered`, `truncate_to_cell_width`, `sanitize_terminal_text`, `UnicodeWidthStr`, and existing Ratatui imports without widening public APIs.
- `src/app.rs:4818-4829` is the source of truth for selection toggling. The UI must render `choice`; it must not maintain a parallel local focus value.
- `src/input/keymap.rs:269-287` is the source of truth for keyboard hints. Keep footer wording synchronized with those keys, but do not modify the keymap in this implementation.
- `src/app.rs:6814-6817` establishes Rollback as the normal default and Abandon as the unknown-outcome default. Rendering tests should create state through `Action::Quit` wherever possible so they verify that contract indirectly.
- Missing tabs currently fall back to `unknown` and `gone`. Preserve those safe fallbacks and never unwrap tab lookup in rendering code; deferred work can race with tab lifecycle events.
- Use `saturating_*` arithmetic for all Ratatui coordinates and widths. The rest of `src/ui/mod.rs` follows this rule to avoid underflow on small terminals.
- Keep action labels short enough for `56 x 16`. Do not add explanatory prose to the normal panel; selected styling already communicates the rollback default.
- Do not reuse `render_profile_group_actions`. Its one-confirm/one-cancel contract does not represent the transaction panel's two mutually exclusive resolution actions.
- Do not snapshot the entire terminal buffer. Semantic text, style, alignment, and boundary assertions are less brittle and match current test conventions.

## Completion Checklist

- [ ] Border title is rendered once.
- [ ] Internal `TRANSACTION SUMMARY` label is present.
- [ ] Current and queued transactions are visually distinguishable.
- [ ] All transaction states use stable user-facing labels and semantic colors.
- [ ] Rollback is selected by default.
- [ ] Commit selection follows the existing choice state.
- [ ] Commit is disabled for aborted transactions.
- [ ] Running queries have a dedicated warning state.
- [ ] Unknown outcomes expose only Abandon and Cancel.
- [ ] Keyboard hints match existing mappings.
- [ ] Minimum terminal size is covered.
- [ ] No transaction reducer, keymap, database, or persistence behavior changed.
- [ ] Focused and full tests pass.
- [ ] Formatting, compilation, and Clippy pass.
