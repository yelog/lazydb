# SQL Editor Lifecycle Design

## Goal

Give workspace tabs explicit close behavior, while distinguishing a temporarily hidden SQL Editor from a permanently deleted SQL Editor. Table preview tabs remain ephemeral and support close only.

## Current Behavior

- `Action::CloseActiveTab` already closes both SQL and relation tabs, but no user key binding invokes it.
- Every `WorkspaceTab::Sql` is included in `workspace_snapshot`, so every SQL Editor is persisted.
- Closing a SQL tab removes it from the snapshot, which currently makes close equivalent to delete.
- `WorkspaceStore::save` rewrites the manifest and current SQL files but never removes orphaned SQL files.
- The app currently preserves at least one SQL Editor, creating an empty editor when the final SQL tab closes.

## Interaction

- `Space q` closes the current tab.
  - A relation/table-preview tab is discarded.
  - A SQL Editor is hidden but remains persisted.
- `Space x` requests permanent deletion of the current SQL Editor.
  - It is a no-op on relation tabs.
  - A confirmation overlay names the editor and states that its persisted record and SQL file will be deleted.
  - `Enter` confirms and `Esc` cancels.
- `Space e` opens a searchable SQL Editor list.
  - The list contains every persisted SQL Editor in stable order.
  - Open editors have an `OPEN` marker.
  - Selecting an open editor activates its tab.
  - Selecting a hidden editor restores and activates its tab.
  - `j`, `k`, arrow keys, text input, Backspace, and `Esc` follow the existing searchable overlay conventions.

## State Model

`tabs` remains the ordered projection of currently open SQL and relation tabs. A separate ordered SQL Editor registry owns persistence identity and metadata for every SQL Editor, including hidden editors.

The registry must not duplicate mutable editor text. SQL text remains in `EditorWorkspace`, keyed by the stable console UUID. Open `WorkspaceTab::Sql` values own transient runtime state. When a SQL Editor closes, its persistent metadata is synchronized to the registry before the tab is removed. Reopening reconstructs a runtime `ConsoleTab` from the registry while preserving the existing editor buffer.

The registry records UUID, display name, execution target, transaction mode, stable order, and open/hidden state.

## Persistence and Safety

The workspace format gains an `open` field on each persisted console and advances its version. Loading the previous format treats every console as open. Saving includes all registry entries and removes only explicitly deleted UUID-specific SQL files. Missing files are treated as already deleted.

Close reuses transaction-exit protection and hides an SQL Editor without deleting its record. Delete requires a confirmation overlay, removes the record, editor session, manifest entry, and exact SQL file, and creates a new empty SQL Editor only when no persisted editor remains.

## Testing

Cover persistence migration, open/hidden round trips, close versus delete, final-editor invariants, reopen/list search, exact file deletion, transaction deferral, keymap, overlay rendering, and documentation.
