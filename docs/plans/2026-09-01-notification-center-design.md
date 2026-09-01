# Notification Center Design

## Objective

Replace the footer's single transient message slot with a unified notification center:

- Render live notifications as independent, non-modal cards stacked from the top-right.
- Include a semantic icon, title, body, timestamp, and visible close affordance.
- Keep every notification in a session history after its live card expires or is dismissed.
- Provide a full notification history overlay with `/` search, `n`/`N` match navigation, and an explicit clear-all action.
- Preserve LazyDB's keyboard-first behavior, small-terminal resilience, icon fallback, and motion setting.

The visual direction borrows the strongest parts of Noice with `nvim-notify` and Snacks: right-aligned compact windows, severity-colored borders and icons, neutral body text, muted time, replacement by stable ID, and history independent from live visibility. It does not copy Neovim-specific floating-window APIs, opacity animation, or unlimited history growth.

## Current Implementation

LazyDB currently has three unrelated message paths.

1. `App::status_message` writes arbitrary validation and operation feedback into `ConnectionState.error` (`src/app.rs:6570`). The footer then renders it in error red (`src/ui/mod.rs:2339`), even when the text is informational or successful, such as `Catalog object dropped`.
2. Clipboard and profile-access feedback use `ClipboardNotice`, a separate success/error model with a fixed two-second TTL (`src/model/clipboard.rs`). The runtime explicitly expires only this model from its 33 ms ticker (`src/runtime.rs:3209`).
3. SQL formatting failures use modal `Overlay::Message` (`src/app.rs:6002-6039`). These messages interrupt input, dim the workspace, and only support generic `Esc`/`q` dismissal.

The second footer row has precedence `clipboard notice -> relation context -> connection.error -> Ready` (`src/ui/mod.rs:2331-2343`). Consequences:

- only one message is visible;
- new feedback silently replaces old feedback;
- there is no timestamp or history;
- most feedback is incorrectly styled as an error;
- persistent `connection.error` can occupy the footer indefinitely;
- modal formatting messages are disproportionate to their severity;
- message behavior depends on which subsystem emitted it.

The existing architecture is otherwise well suited to this feature:

- Ratatui supports layered rendering with `Clear` and bordered blocks.
- the runtime already ticks every 33 ms;
- `tachyonfx` and the app's reduced-motion configuration already exist;
- overlays already receive input before the workspace;
- search state patterns already exist in Explorer and Help;
- `unicode-width` and icon fallback are already dependencies.

## Chosen Architecture

Add one app-owned `NotificationCenter`. All user-facing transient feedback enters it through a small semantic API. The center owns both immutable history entries and live-card lifecycle state; renderers receive read-only projections.

```text
App reducer / runtime result
        |
        v
NotificationCenter::push(level, title, body, options)
        |                         |
        |                         +--> visible cards (bounded, expiring)
        +----------------------------> session history (bounded ring)
                                          |
                                          +--> NotificationHistory overlay
```

This follows Noice's most important design decision: live display and history share one message model, while dismissal/timeout affects visibility rather than deleting history.

Do not introduce a general event bus or pluggable backend abstraction. LazyDB has one Ratatui renderer and no concrete need for multiple notification backends. A direct app-owned model is smaller, easier to test, and consistent with the current reducer architecture.

## Data Model

Create `src/model/notification.rs` with these concepts:

```rust
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

pub struct Notification {
    pub id: u64,
    pub level: NotificationLevel,
    pub title: String,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Local>,
    pub source: Option<NotificationSource>,
}

pub struct LiveNotification {
    pub notification_id: u64,
    pub expires_at: Option<Instant>,
    pub phase: NotificationPhase,
    pub animation_started_at: Instant,
}

pub struct NotificationCenter {
    history: VecDeque<Notification>,
    live: Vec<LiveNotification>,
    next_id: u64,
}
```

`NotificationSource` should be a small enum only if the source is displayed or filtered in the first version, for example `Connection`, `Query`, `Catalog`, `Clipboard`, `Profile`, and `Editor`. Do not store arbitrary subsystem strings.

Defaults:

| Level | Icon intent | TTL | Ordering |
| --- | --- | --- | --- |
| Success | check | 3 s | after warnings/errors |
| Info | information | 4 s | after success |
| Warning | warning | 6 s | before success/info |
| Error | failure | 8 s | first |

Errors should not be permanently sticky by default. Eight seconds plus permanent history is discoverable without leaving stale cards on screen. A future operation-progress notification can opt into `expires_at: None`, but this feature should not add progress support speculatively.

Use these limits:

- history capacity: 500 entries;
- visible cards: at most 4;
- title: one rendered line;
- body stored up to 16 KiB, with an explicit truncation suffix if needed;
- duplicate coalescing window: optional, only for exact same level/title/body within one second.

The center exposes narrow methods:

```rust
push(level, title, body, now) -> NotificationId
dismiss_live(id)
dismiss_all_live()
expire(now) -> bool
clear_history()
history() -> impl DoubleEndedIterator<Item = &Notification>
```

Stable replacement IDs, source filtering, persistence across restarts, and per-level configuration are valuable later but are not required for the first implementation. Keep IDs internal and monotonic so replacement can be added without migrating stored state.

## State Ownership And Migration

Add `pub notifications: NotificationCenter` to `App` and remove `clipboard_notice` after all call sites migrate.

Split two meanings currently combined in `ConnectionState.error`:

- connection-domain failure state remains in `connection.error`, because Explorer and connection recovery logic depend on it;
- user feedback is emitted to `NotificationCenter` with the correct level.

`App::status_message` should be replaced with semantic helpers rather than retained as an error-colored alias:

```rust
fn notify_info(&mut self, title: &str, body: impl Into<String>)
fn notify_success(&mut self, title: &str, body: impl Into<String>)
fn notify_warning(&mut self, title: &str, body: impl Into<String>)
fn notify_error(&mut self, title: &str, body: impl Into<String>)
```

Four private helpers are acceptable because they prevent the current classification bug at call sites. They should delegate immediately to one center method and not contain subsystem logic.

Migration rules:

| Current feedback | New level/title |
| --- | --- |
| guard rejected because context is unavailable | Warning / action domain |
| operation completed, copied, saved, dropped | Success / action domain |
| non-fatal unavailable feature or stale result discarded | Warning |
| runtime/database/clipboard operation failed | Error |
| neutral state transition such as disconnected | Info or no notification |
| actionable connection failure | Keep `connection.error` and also emit Error once |
| SQL format no scope/unsupported visual block | Warning / `SQL FORMAT` |
| SQL formatter/editor replacement failure | Error / `SQL FORMAT` |

Do not mechanically map every existing `status_message` to Error. Each of its call sites must be classified during implementation. This is the largest correctness-sensitive part of the migration.

## Live Notification Cards

### Placement

Render cards after the base workspace and completion popup, but before modal dimming/effects. When a modal overlay is open, keep live cards visible above the dimmed workspace but below the modal, or pause their TTL and render them after dimming with muted contrast. The preferred behavior is:

1. base workspace;
2. live notifications;
3. modal dim layer;
4. modal overlay.

This avoids notifications competing visually with confirmations. TTL is paused while a modal overlay is open so a message cannot expire unseen.

Use the frame area rather than pane geometry:

- top margin: header height plus one row;
- right margin: one column;
- bottom boundary: footer start;
- gap: one row between cards;
- stack direction: top to bottom;
- ordering: errors, warnings, success, info; preserve creation order within a level.

If fewer than six usable rows remain, do not render cards; history remains available. If cards do not fit, render the highest-priority cards and a compact final card such as `+3 more  Space m: history`.

### Width And Height

Measure terminal display columns with `UnicodeWidthStr`, not byte or character length.

```text
min width: 34
preferred width: longest wrapped line + padding
max width: min(64, 42% of frame width)
max card height: min(10, 35% of usable height)
```

On terminals narrower than 70 columns, use up to `frame.width - 4`. Clamp width first, wrap the body against the final inner width, then calculate height. Long bodies end with a muted `... N more lines` footer and remain fully available in history.

### Visual Language

Use Noice/nvim-notify's `fancy` information hierarchy while adapting it to LazyDB's Deep Space palette.

```text
╭─ ✓ SUCCESS · CATALOG ───────────── 14:32:08  × ╮
│  Table public.users dropped successfully       │
╰─────────────────────────────────────────────────╯

╭─ ! WARNING · QUERY ──────────────── 14:32:11  × ╮
│  Select an execution target before running SQL  │
╰──────────────────────────────────────────────────╯
```

The exact title can be split into border title and right-side title spans if Ratatui's block title alignment permits both. Otherwise render a header row inside a rounded bordered block; reliability is more important than putting time literally in the border.

Color only semantic anchors:

- icon, border, and level label use the level color;
- title and body use `theme.text`;
- timestamp and close glyph use `theme.muted`;
- card body uses `theme.surface_raised`;
- no full red/yellow/green backgrounds;
- selected/hovered close target uses `theme.selection`.

Extend `Theme` with one semantic success color rather than overloading `accent`:

```text
success: #76E6A6
info: existing action blue #65A7FF
warning: existing amber #F4B860
error: existing coral #FF6B7A
```

Keep cyan `accent` for focus and product chrome. This yields a richer palette without confusing focus with success.

Icons must use the existing `IconSet` mode:

| Level | Nerd Font | Unicode/ASCII fallback |
| --- | --- | --- |
| Success | check-circle | `OK` or `+` |
| Info | info-circle | `i` |
| Warning | warning-triangle | `!` |
| Error | circle-x | `x` |
| Close | close | `x` |

Never rely on color or glyph alone; always render the textual level.

### Close Interaction

The visible `x` is primarily a mouse hit target. Add `HitTarget::DismissNotification(id)` for each card header/close cell and map a click to `Action::DismissNotification(id)`.

Keyboard users should not have to focus transient cards. Provide:

- `Esc`: dismiss only the newest visible card when no modal, editor insert state, completion, or active search owns Escape;
- `Space m`: open history from Explorer/Results application leader;
- editor users open history through an explicit editor-leader binding or global function key shortcut chosen consistently with modalkit; recommended global fallback is `F8`.

Because global `Esc` currently has editor/search semantics, implementing `Esc` dismissal is optional and must not steal those contexts. The close glyph plus history shortcut satisfies the explicit close requirement without destabilizing modal editing.

## Motion

Use the existing motion mode and 33 ms ticker; do not introduce a second timer.

Default animation:

- enter: 120 ms, slide from four columns outside the right edge and reveal the border in 3-4 frames;
- exit on dismiss/expiry: 90 ms, contract by two columns or remove immediately if contraction harms text stability;
- stack reflow: immediate, not spring based;
- reduced motion: render final geometry immediately.

Do not emulate nvim-notify's 30 FPS opacity interpolation. Terminal alpha is unreliable, high-frequency full-screen redraws are expensive over SSH/tmux, and `tachyonfx` cannot make transparent cells portable. A short positional reveal gives the desired polish at much lower cost.

`NotificationCenter::expire(now)` advances lifecycle and returns whether a redraw is needed. `UiState` may own purely visual interpolation, but notification identity and visibility remain in `App`; tests must not depend on frame timing.

## History Overlay

Add `Overlay::NotificationHistory(NotificationHistoryState)` and render it as a large centered workspace:

```text
╭─ NOTIFICATION CENTER ─────────────────────────────  128 events ─╮
│ / connection refused_                     [ALL] [ERR 4] [WARN 8]│
├─────────────────────────────────────────────────────────────────┤
│ 14:32:11  ! WARN   QUERY      Select an execution target        │
│ 14:31:58  x ERROR  CONNECTION connection refused                │
│ 14:31:44  ✓ OK     CLIPBOARD  Copied row with headers           │
│ ...                                                             │
├─────────────────────────────────────────────────────────────────┤
│ x ERROR · CONNECTION                             14:31:58       │
│ connection refused                                            │
│ host=localhost port=5432                                      │
├─────────────────────────────────────────────────────────────────┤
│ / search  n/N match  j/k select  Enter detail  c clear  Esc close│
╰─────────────────────────────────────────────────────────────────╯
```

At widths of 100 columns or more, use a 58/42 split: list on the left and selected-message detail on the right. At narrower widths, use a vertical list/detail split. Below approximately 60x16, degrade to a list-only popup and open the full selected body with Enter.

The overlay must derive viewport start from selection and available list height, as Help does, rather than storing terminal-dependent offsets in the app model.

## Search Model

Create `NotificationHistoryState` in the notification model:

```rust
pub enum HistorySearchPhase {
    Inactive,
    Editing,
    Confirmed,
}

pub struct NotificationHistoryState {
    pub query: String,
    pub phase: HistorySearchPhase,
    pub selected: usize,
    pub active_match: usize,
}
```

Semantics:

- overlay opens with the newest entry selected and search inactive;
- `/` enters Editing and places a bar cursor in the search field;
- printable characters append to the query;
- Backspace edits; `Ctrl-u` clears;
- Enter confirms the search and selects the first match;
- while Confirmed, `n` moves to the next match and `N` to the previous, wrapping at both ends;
- `/` from Confirmed starts a new edit using the existing query;
- `Esc` while Editing cancels the edit and restores the previous confirmed query; a second `Esc` closes the overlay;
- `j/k`, arrows, Home/End move the selected history row when not Editing;
- query matching is case-insensitive substring matching across title, body, textual level, source, and rendered local time;
- newest-first ordering never changes during search.

Do not add fuzzy scoring or regex syntax initially. The user explicitly asked for keyword search with Vim navigation; stable substring matches are predictable, simple, and match existing Explorer behavior.

Unlike Explorer's confirmed search, filtered-out rows should remain visible but muted only if `n/N` navigation is the sole objective. The preferred design is instead to filter the list to matching entries while Editing/Confirmed, because this makes history browsing useful and gives a clear result count. `n/N` then cycles through the filtered entries and centers the active one. The search header displays `3/12`.

## Clear-All Interaction

Bind lowercase `c` in history browse mode to start an inline confirmation footer:

```text
Clear all 128 notifications?  y confirm  n/Esc cancel
```

Represent this with `confirm_clear: bool` inside `NotificationHistoryState`; do not open a nested overlay because `App` supports one overlay and nested modal state would complicate dismissal.

On confirm:

- clear history and live cards atomically;
- reset query, selection, and confirmation state;
- keep the history overlay open with an empty-state illustration/message;
- do not emit a `History cleared` notification, which would immediately repopulate the list.

Empty state:

```text
               NO SIGNALS YET
     Runtime events will collect here.
```

Use a subdued icon/radar-line ornament only when icons are enabled. Keep it decorative and no more than three lines so the UI remains usable in short terminals.

## Footer After Migration

Remove transient messages from the second footer row. Preserve relation DDL context there; otherwise show a stable workspace status such as connection target, row/column context, or `Ready`.

Add a compact unread/history indicator to the first footer row when space permits:

```text
F8 messages 3
```

The count should represent currently visible or unseen-since-history-open notifications, not total history. If unread semantics are not implemented in the first pass, show `F8 messages` without a misleading count.

## Input Priority

History-specific mapping must run before generic overlay handling in `Keymap::map`, following Help and Explorer search patterns.

Priority while history is open:

1. global `Ctrl-c` quit remains first, matching current behavior;
2. clear confirmation keys;
3. search Editing keys;
4. confirmed-search `n/N` and `/`;
5. browse navigation and clear;
6. `Esc`/`q` closes the history overlay.

Printable `q`, `j`, `k`, `n`, and `N` belong to the query while Editing. Paste should append sanitized single-line text through a dedicated history paste action, matching Help's established behavior.

Do not assign bare `m` globally: it can conflict with editor marks and future grid commands. Prefer an application leader sequence plus one globally discoverable function key.

## File-Level Change Map

### New files

- `src/model/notification.rs`
  - levels, entries, center lifecycle, history search state, matching, bounded retention, unit tests.
- `src/ui/notifications.rs`
  - live-card measurement/wrapping/placement, history overlay, narrow-terminal degradation, render tests/helpers.

### Modified files

- `src/model/mod.rs`
  - export the notification model.
- `src/model/workspace.rs`
  - add `Overlay::NotificationHistory(NotificationHistoryState)`; retain `Overlay::Message` only for genuinely modal information, or remove it after SQL formatting migration.
- `src/app.rs`
  - add the center; add semantic notify helpers; implement open/search/navigation/dismiss/clear actions; migrate `ClipboardNotice`, `status_message`, and non-modal `Overlay::Message` call sites; expire notifications from one method.
- `src/action.rs`
  - add open/dismiss/history-search/history-navigation/clear-confirmation actions.
- `src/input/keymap.rs`
  - map history overlay first; add history opening shortcut; protect editor/search ownership of keys; add focused keymap tests.
- `src/input/mouse.rs`
  - map card close hit targets and optionally history rows.
- `src/ui/mod.rs`
  - invoke live-card rendering in the correct layer; delegate history overlay rendering; remove footer transient-message precedence; add hit targets and overlay key.
- `src/ui/theme.rs`
  - add semantic success color and notification styles.
- `src/ui/icons.rs`
  - add semantic notification/close icons with existing icon-mode fallbacks.
- `src/runtime.rs`
  - replace `expire_clipboard_notice` with notification expiry/lifecycle redraw; no new ticker.
- `src/model/clipboard.rs`
  - delete after all call sites migrate.
- `src/help.rs`
  - add the notification-center shortcut to the shared catalog and footer hints.
- `docs/keybindings.md`
  - document live dismissal, history, search, navigation, and clear confirmation.
- `tests/ui_render.rs`
  - cover card geometry, stacking, colors/content, history layouts, search cursor, and small-terminal fallback.
- relevant reducer integration tests under `tests/`
  - assert classification and history retention for representative connection, query, clipboard, catalog, and editor outcomes.

## Implementation Sequence

### Phase 1: Model First

1. Add failing unit tests for push, severity TTL, expiry without history deletion, live dismissal, capacity, and clear-all.
2. Implement `NotificationCenter` and history matching.
3. Add reducer actions and state transitions for history search/navigation/confirmation.
4. Verify model and reducer tests before rendering.

### Phase 2: Live Cards

1. Add render tests for one success card with icon/body/time/close affordance.
2. Add tests for severity ordering, four-card cap, overflow card, Unicode wrapping, long body clipping, and tiny terminals.
3. Implement `src/ui/notifications.rs` and top-right placement.
4. Add mouse close hit regions.
5. Connect expiry to the existing ticker and motion mode.

### Phase 3: History Overlay

1. Add keymap tests for `/`, text input, Backspace, `Ctrl-u`, Enter, `n/N`, navigation, clear confirmation, and `Esc` priority.
2. Add render tests for wide, narrow, empty, searched, selected, and clear-confirmation states.
3. Implement responsive list/detail rendering and search cursor.
4. Add paste support and help catalog entries.

### Phase 4: Message Migration

1. Migrate `ClipboardNotice` callers and delete its expiry path.
2. Audit every `status_message` call and classify it; do not bulk-convert.
3. Keep actual connection errors in connection state while emitting one notification at the transition boundary.
4. Convert SQL formatting `Overlay::Message` uses to warning/error notifications.
5. Remove transient footer rendering and delete unused model code.

### Phase 5: Polish And Verification

1. Verify default icons and ASCII fallback snapshots.
2. Verify motion on/off and that modals pause notification expiry.
3. Update `docs/keybindings.md` and contextual Help.
4. Run `cargo fmt --check`.
5. Run focused notification, keymap, reducer, and UI tests.
6. Run `cargo test`.
7. Run `cargo clippy --all-targets --all-features -- -D warnings`.

## Test Matrix

### Model

- each level receives the expected TTL;
- expiry removes only live state;
- dismiss one and dismiss all preserve history;
- clear removes history and live state;
- 501 pushes retain the newest 500;
- same timestamps preserve deterministic ID order;
- case-insensitive search covers title, body, level, source, and time;
- `n/N` wrap and remain valid after clear or capacity eviction.

### Reducer And Input

- representative success/warning/error actions produce correctly classified entries;
- connection failure is not duplicated on stale runtime events;
- history opens from every supported context without stealing editor marks/input;
- search Editing owns printable keys;
- confirmed search owns `n/N`;
- clear requires confirmation and emits no replacement notification;
- generic overlay dismissal does not bypass history search cancellation;
- paste is single-line and Unicode-safe.

### Rendering

- cards occupy the top-right usable area and never cover header/footer;
- cards do not overlap and preserve one-row gaps;
- severity changes icon/border/label but not body color;
- timestamp and close glyph are visible;
- CJK, combining text, and fallback icons wrap by display width;
- overflow count is correct;
- modal overlays remain visually dominant;
- wide history uses list/detail columns;
- narrow history uses vertical or list-only layout;
- selected match remains visible and search cursor is a bar;
- empty state and clear confirmation fit a short terminal.

## Acceptance Criteria

- No transient operational message is rendered in the footer.
- Every newly emitted notification has a level, title, body, local timestamp, and history entry.
- Up to four live cards stack from the top-right without blocking keyboard input.
- Each card visibly exposes a close glyph and can be dismissed with mouse when mouse mode is enabled.
- Live expiry/dismissal never deletes history.
- The history overlay shows all retained messages newest first with icon, content, and time.
- `/` edits a keyword query; Enter confirms; `n/N` wrap through matches.
- Clear-all requires confirmation and clears live plus history atomically.
- Rendering remains safe and useful at small terminal sizes and with ASCII icons.
- Existing editor, Explorer search, completion, modal, and `Ctrl-c` behavior does not regress.
- No new production dependency is added.

## Key Design Decisions

1. **Unify before beautifying.** Moving `connection.error` to a floating box without a semantic message model would preserve incorrect severity, replacement, and missing-history bugs.
2. **History is authoritative.** Live cards are a temporary projection, exactly as Noice keeps history independently of notifier windows.
3. **Polish semantic anchors, not the whole card.** Colored borders/icons and a neutral body fit Deep Space and stay readable when several cards stack.
4. **Use restrained terminal motion.** A 120 ms slide/reveal captures the visual energy of nvim-notify without fragile alpha or spring animation.
5. **Search is predictable.** Stable case-insensitive substring filtering plus Vim `n/N` navigation is preferable to fuzzy reordering for an audit-style history.
6. **Clear is explicit.** Clearing is destructive to session history, so an inline confirmation is required even though no persistent storage is involved.
7. **No persistence yet.** The request asks for message history, which is best interpreted as current-session history. Disk persistence introduces privacy, retention, migration, and sensitive SQL/error-content concerns and should be designed separately if needed.
