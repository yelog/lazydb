# Read-Only Vim Copy Design

- Status: Approved
- Date: 2026-08-31
- Scope: Explorer node copy, SQL Output Log navigation and copy, relation DDL navigation and copy

## Goal

Make currently non-copyable text keyboard-accessible without introducing separate navigation and selection implementations. Explorer copies the selected node's semantic name. SQL Output Log and relation DDL reuse the SQL Editor's Vim cursor, selection, viewport, and yank behavior through strongly read-only editor sessions.

## Current State

- Explorer tracks a selected tree node but has no semantic copy action.
- SQL Output Log renders the last visible `OutputEntry` values directly from a list. It has neither a cursor nor a selection model.
- Relation DDL has independent horizontal and vertical scroll state but no cursor or selection model.
- `EditorWorkspace` already provides Normal and Visual modes, Vim motions, registers, search, viewport management, Unicode-aware render snapshots, and clipboard yank effects.
- Existing clipboard writes flow through `Command::WriteClipboard` and report success or failure through `ClipboardNotice`.

## Decisions

### Explorer Copy

Pressing `y` in the normal Explorer browsing state copies the selected node's primary semantic name:

- Profiles copy the profile name.
- Databases, schemas, relations, columns, indexes, constraints, routines, and other catalog entries copy their primary object name.
- Presentation groups copy their group label.
- Operational rows such as loading, empty, error/retry, and load-more rows do not copy placeholder text and do not replace the clipboard.
- Explorer search input retains input priority, so typing `y` while editing a search query inserts `y` rather than copying a node.

Icons, indentation, type details, comments, and status decorations are never included. The action uses the existing clipboard command and notice flow.

### Read-Only Editor Sessions

`EditorWorkspace` gains an explicit session capability:

- `Editable` for SQL console buffers.
- `ReadOnly` for derived Output Log and DDL text.

Both capabilities use the same Modalkit buffer, key manager, cursor, viewport, selection, registers, and `EditorRenderSnapshot`. Read-only is enforced before applying editor actions. The implementation must not mutate and then restore text because that would contaminate revision, undo, dot-repeat, register, and selection state.

Read-only sessions allow:

- Normal-mode navigation, including `h/j/k/l`, arrows, `0/$`, word motions, `gg/G`, `H/M/L`, `Ctrl-u/d/f/b`, and supported counts.
- Search through `/`, `?`, `n`, and `N`.
- Visual Char, Visual Line, and Visual Block selection through `v`, `V`, and `Ctrl-v`.
- Yank operations, including Visual `y`, `yy`, and supported yank-plus-motion sequences.
- Escape and non-mutating viewport operations.

Read-only sessions silently reject every mutation path, including Insert and Replace entry, delete, change, paste, undo, redo, dot-repeat of edits, substitution, and write-oriented Ex commands. SQL-specific effects such as run, format, transaction control, console lifecycle, and target selection are not emitted by read-only sessions.

### Session Ownership

Business models remain authoritative:

- `ConsoleTab.output` remains the Output Log data source.
- The adapter-owned relation DDL snapshot remains the DDL data source.
- Read-only buffers are interaction projections and never write back to either model.

Each SQL console has a distinct Output Log read-only session. Each relation tab has a distinct DDL read-only session. These sessions do not share identifiers, cursor state, selection state, or viewport state with editable SQL sessions. They follow the owning tab lifecycle and are closed when their owner is permanently removed.

### Output Log Projection

Output text is formed from `OutputEntry.message` values in append order. Status markers such as `·`, `✓`, `!`, and `×` remain visual decorations and are not part of selectable or copied text. Embedded newlines are preserved as real buffer lines.

An untouched Output Log follows newly appended output. Once the user navigates or starts a selection, subsequent output is appended without moving the cursor, selection, or viewport. Returning to the end through normal navigation does not need a separate follow-mode command in this increment.

The renderer reuses the read-only editor snapshot for text, cursor, selections, horizontal offset, and vertical viewport. Entry kinds may still style their corresponding projected lines without inserting marker characters into the buffer.

### Relation DDL Projection

The complete adapter-owned DDL string is projected into the relation tab's DDL read-only session. DDL refresh updates the same session. Cursor, selection, and viewport coordinates are retained where valid and clamped safely when replacement text is shorter.

The DDL page no longer has an independent keyboard scroll model once the read-only session is authoritative. Mouse-wheel scrolling also updates the read-only editor viewport.

### Input Priority

Existing application navigation remains authoritative before read-only Vim input:

- Overlay and focused text-input handling.
- Global pane and tab navigation.
- Existing Output and relation page switching such as `o`, `1`, `2`, and `3`.
- Relation refresh and cancellation actions.
- Read-only Vim navigation, search, selection, and yank.

This preserves current product shortcuts instead of assigning Vim's editing meaning to keys such as `o` inside read-only views.

### Clipboard Behavior

All semantic copies and yanks use `Command::WriteClipboard`. Explorer descriptions identify the copied node. Output and DDL yanks describe the copied selection and character count. Successful and failed writes continue to use the existing transient clipboard notice.

The unnamed and named Vim registers remain shared within `EditorWorkspace`, matching existing SQL Editor behavior. The system clipboard receives successful user yanks through the reducer/runtime command boundary rather than direct UI or editor I/O.

## Alternatives Considered

### Independent Read-Only Vim Component

A separate component could implement only the requested motions and selection modes. This avoids touching SQL Editor internals but duplicates cursor, selection, Unicode-width, viewport, register, search, and key-sequence logic. Its behavior would drift from SQL Editor and every future Vim improvement would require parallel work.

### Reuse an Editable SQL Session Directly

Output or DDL could be loaded into ordinary SQL editor buffers. This maximizes immediate reuse but conflates derived text with persisted SQL documents and exposes run, format, transaction, undo, and mutation behavior. It also creates unclear ownership when output is appended or DDL is refreshed.

The explicit read-only session capability is preferred because it reuses interaction machinery while preserving domain ownership and enforcing immutability at the correct boundary.

## Error Handling

- A missing or stale read-only session results in no mutation and an actionable internal/editor error path rather than a panic.
- Empty Output and unavailable DDL retain their existing empty/loading/error presentation. No synthetic placeholder text is inserted into selectable buffers.
- DDL refresh clamps stale cursor and viewport coordinates.
- Unsupported or mutation-oriented Vim actions in read-only sessions are ignored without a notice.
- Clipboard backend failures use the existing failure notice and leave the source selection unchanged.

## Verification

### Editor Contract Tests

- Read-only sessions support Normal, Visual Char, Visual Line, and Visual Block modes.
- `h/j/k/l`, `H/M/L`, `gg/G`, `Ctrl-u/d/f/b`, counts, search, and representative word/line motions produce the same cursor behavior as editable sessions.
- Visual `y`, `yy`, and yank-plus-motion update registers and emit the clipboard yank effect.
- Insert, Replace, delete, change, paste, undo, redo, dot-repeat, and substitution cannot change read-only text or its revision.
- CJK, emoji, combining characters, tabs, long lines, and multiline selections render and copy correctly.

### Keymap and Reducer Tests

- Explorer `y` copies primary names for profiles, groups, relations, and columns.
- Operational Explorer rows do not emit clipboard writes.
- Explorer search input consumes `y` while editing.
- Output and DDL view-switching shortcuts take priority over read-only Vim input.
- Output appends follow the tail before interaction and preserve cursor/selection after interaction.
- DDL refresh preserves valid positions and clamps invalid positions.
- Read-only yanks produce `Command::WriteClipboard` and clipboard notices.

### UI Tests

- Output and DDL show a cursor only when their page has focus.
- Visual selections use the SQL Editor's selection-cell styling.
- Output status markers remain visible but are outside selectable text.
- DDL provenance and status presentation remain visible and outside selectable text.
- Rendering is covered at 120x36, 80x24, and the smallest supported layout, including wide Unicode characters and horizontal scrolling.

### Completion Gate

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Update `docs/keybindings.md` to document Explorer copy and read-only Vim navigation, selection, and yank behavior for Output Log and relation DDL.

## Non-Goals

- Editing Output Log or relation DDL.
- Copying Explorer icons, details, comments, or complete rendered rows.
- Making status markers or DDL provenance text selectable.
- Persisting Output/DDL cursor and selection state across application restarts.
- Adding a separate follow-tail toggle command.
- Expanding Vim compatibility beyond behavior already supported by the shared editor engine.
