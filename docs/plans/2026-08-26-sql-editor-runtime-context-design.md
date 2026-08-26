# SQL Editor, Transactions, and Execution Context Design

**Status:** Approved

**Date:** 2026-08-26

## Summary

LazyDB will make modalkit the single authority for each SQL editor's mode,
cursor, selection, pending operations, and edit history. This removes the
current split between modalkit state and `EditorSession` shadow state that makes
Normal-mode commands insert literal characters and breaks text objects, Visual
mode, undo, and cursor placement.

The transaction implementation will be made safe and repeatable before its
controls are exposed prominently. Each SQL editor will then display and control
its AUTO or MANUAL transaction mode. Runtime transaction ownership will be
validated by complete connection and transaction identity, and uncertain
commit or rollback outcomes will remain explicit.

Every SQL editor will bind to a persisted execution target containing a
connection profile, database, and schema. Activating a tab will safely switch
the application's single active pool to that target. The complete workspace,
including tabs, SQL text, targets, active tab, and transaction mode preference,
will be restored across launches without persisting results, secrets, or active
transaction state.

## Confirmed Decisions

- Keep modalkit and make it the sole editor state authority.
- Give each console independent modal, prefix, selection, and history state.
- Keep registers and command history shareable across consoles.
- Preserve `App::update` as the only application-state mutation boundary.
- Fix transaction safety and lifecycle defects before making controls prominent.
- Bind every SQL editor to a connection profile, database, and schema.
- Keep one active runtime pool and safely auto-switch it when activating tabs.
- Fail closed when the active runtime target does not exactly match the editor.
- Persist the complete workspace using a manifest and separate SQL files.
- Restore MANUAL as a preference only; all restored transactions start Idle.
- Use a status-bar selector and Ex commands as the primary target controls.

## Goals

- Make documented Vim motions, operators, text objects, Visual modes, undo, and
  redo work through the real input pipeline.
- Distinguish Normal, Insert, Replace, Visual, and prompt cursor styles.
- Place the cursor after accepted completion text and include completion in undo
  history.
- Show truthful transaction mode and state and provide discoverable controls.
- Make repeated MANUAL transactions safe on a pinned physical connection.
- Show and persist each editor's exact execution profile, database, and schema.
- Restore tabs and SQL text without persisting sensitive or transient state.
- Reject stale asynchronous results and mismatched execution identities.

## Non-goals

- Implementing a new Vim engine.
- Keeping multiple database pools active simultaneously.
- Restoring an active or uncommitted database transaction after restart.
- Persisting query results, output logs, completion state, selections, or undo
  history.
- Treating catalog visibility as database authorization.
- Supporting arbitrary PostgreSQL cross-database execution on one connection.

## Current Root Causes

### Split editor authority

`EditorSession` stores a shadow mode and cursor while modalkit maintains the
actual modal state and buffer cursor. Insert input is manually applied by
replacing the complete buffer with `set_text`, which resets modalkit cursor and
history state. `Esc` and `i` are intercepted without updating modalkit. Pending
operator modes are projected as Normal and their next key can be stolen by the
manual Insert handling.

As a result, many Vim operations exist in modalkit but are unreachable or start
from stale state. Existing tests often call internal editor methods directly and
therefore do not exercise this production failure.

### Completion cursor policy

Completion uses a generic range replacement that places the cursor at the range
start. It also replaces the full buffer, disconnecting the insertion from the
real edit history.

### Incomplete transaction integration

Transaction models, actions, SQL classification, and pinned workers exist, but
some editor effects are discarded. The current exit confirmation can display
Rollback while dispatching Commit, scoped savepoint execution can reread the
whole buffer, BEGIN can be reported active before acknowledgement, and terminal
workers are not reliably removed from the runtime registry. Identity,
cancellation, shutdown, and unknown-outcome paths are also incomplete.

### No editor-local execution target or workspace persistence

Execution uses one global active connection and profile defaults. `ConsoleTab`
does not own a target, and `PersistWorkspace` has no runtime implementation.
Changing tabs cannot guarantee that the displayed editor and active pool refer
to the same database.

## Editor Architecture

### Single authority

`EditorWorkspace` remains the boundary that hides modalkit types from the rest
of the application. Within that boundary, each console owns one authoritative
editor state containing:

- text buffer and leader cursor;
- mode and pending operator state;
- count and key-prefix state;
- selection and Visual shape;
- undo and redo history;
- repeat state;
- viewport state required by editor actions.

No second mode, cursor, selection, or history is maintained for display. Domain
snapshots project modalkit state for the App and UI. Shared registers and prompt
history may live in a shared store, but modal and prefix state must not leak
between consoles.

### Input flow

Every editor key, including printable Insert input, `Esc`, `i`, arrows, Tab, and
control keys, enters the same modal key machine:

```text
Crossterm event
  -> Keymap
  -> Action::EditorKey
  -> App::update
  -> EditorWorkspace
  -> per-console modalkit machine
  -> domain EditorEffect values
```

Normal typing must use modalkit insertion actions rather than whole-buffer
`set_text`. Full replacement is reserved for loading a document or an explicit
programmatic transformation.

The editor adapter drains all supported action classes, including editor edits,
repeat, scrolling, jumps, macros, prompts, and application leader commands.
Unsupported actions produce a visible diagnostic instead of being discarded.

### Edit finalization

One finalization path runs after each logical editor operation. It synchronizes
the domain projection and emits at most one logical change notification. It
updates:

- document revision;
- mode and cursor projection;
- selection projection;
- undo grouping;
- syntax-highlight invalidation;
- completion invalidation and scheduling;
- `EditorEffect::Changed`.

### Visual rendering

Visual rendering uses the actual selection shape:

- Visual Char highlights exact character ranges.
- Visual Line highlights complete logical lines.
- Visual Block highlights a common display-cell interval on each selected line.

Screen projection must account for Unicode scalar boundaries, grapheme display
width, CJK, emoji, and tabs. UTF-8 byte offsets are not terminal columns.

### Cursor style

UI rendering exposes a domain cursor style through `UiState`; the terminal layer
applies Crossterm cursor-style effects and restores the default during teardown.

| Editor state | Cursor style |
| --- | --- |
| Normal | Block |
| Insert | Bar |
| Replace | Underline or thick bar |
| Visual Char/Line/Block | Block |
| Search or Ex prompt | Bar |

Opening a modal overlay or leaving editor focus updates cursor visibility and
style consistently.

### Programmatic replacement

Range replacement takes an explicit cursor policy:

```rust
enum ReplacementCursor {
    Start,
    EndOfInsertion,
    PreserveRelative,
}
```

Completion uses `EndOfInsertion`. Formatting and substitution choose their
policy explicitly. Completion acceptance is one undoable edit, so accepting
`SELECT` produces `SELECT|`, continuing input produces `SELECT |`, and one undo
can reverse the accepted completion according to the editor's undo grouping.

## Transaction Architecture

### Per-console state

Each console retains the following visible states:

```text
AUTO
MANUAL:IDLE
MANUAL:STARTING
MANUAL:ACTIVE
MANUAL:ABORTED
MANUAL:COMMITTING
MANUAL:ROLLING_BACK
MANUAL:OUTCOME_UNKNOWN
```

AUTO uses pool execution. MANUAL:IDLE records the preference without occupying
a physical connection. The first statement lazily begins a pinned transaction.
ABORTED permits rollback only. OUTCOME_UNKNOWN never claims that commit or
rollback succeeded or failed.

Workspace restoration maps every persisted MANUAL mode to MANUAL:IDLE.

### Semantic controls

Keyboard, Ex commands, and mouse controls emit the same semantic actions:

```text
Space t t    Toggle AUTO / MANUAL
Space t c    Commit
Space t r    Rollback

:tx auto
:tx manual
:commit
:rollback
:tx clear
```

Editor effects for toggling and clearing transaction outcome must be translated
by the App rather than discarded.

AUTO to MANUAL changes preference immediately but begins lazily. MANUAL to AUTO
is immediate only from Idle. Active or Aborted state requires an exit prompt
whose default is Rollback. Enter executes the visibly selected typed choice;
Rollback, Commit, and Cancel cannot collapse into one choice-free action.

Commit is available only from Active and rollback from Active or Aborted. No
transaction-control command can pass a running query. Cancellation and rollback
are one ordered flow rather than concurrent worker commands.

### Immutable SQL policy

All SQL, including transaction control, first becomes an immutable execution
draft with exact scope and identity. Savepoint execution sends the classified
scope snapshot and never rereads the complete editor buffer. Mixed transaction
control and data remain rejected until a separate batch policy is designed.

`BEGIN`, `COMMIT`, and `ROLLBACK` route to the same semantic transaction paths as
the UI controls. Execution checks query state and stale identity before changing
the visible transaction state.

### Runtime ownership

The pinned transaction registry validates this complete identity:

```text
console_id
+ profile_id
+ connection_generation
+ transaction_generation
```

Query generation is additionally validated for execution and cancellation.
Runtime performs the following sequence:

1. Acquire one physical connection.
2. Execute and acknowledge BEGIN.
3. Emit `ManualStarted` only after acknowledgement.
4. Serialize SQL, commit, rollback, and cancellation on that worker.
5. Remove the matching registry entry at every terminal disposition.
6. Reject stale commands without affecting a newer transaction.
7. Retain the real forced-close handle and await it during shutdown.

The same console must be able to begin another transaction after a successful
commit or rollback.

### Errors and uncertain outcomes

Transaction errors retain structured information until the App chooses a state:

- error category and database code;
- whether the connection remains trustworthy;
- whether the operation was sent;
- whether the server acknowledged the outcome.

A confirmed BEGIN failure returns to MANUAL:IDLE. PostgreSQL statement failures
normally enter ABORTED. MySQL and SQLite statement failures may remain Active
when the connection and transaction are known to be valid. Connection loss after
a commit request was sent enters OUTCOME_UNKNOWN.

OUTCOME_UNKNOWN is cleared only by an explicit verify/reconnect flow. It does not
offer ordinary Commit or Rollback actions.

### Lifecycle gate

One App-level lifecycle gate protects:

- closing a console;
- changing its profile, database, or schema;
- switching MANUAL to AUTO;
- editing, deleting, switching, or disconnecting its profile;
- quitting the application.

Active transactions offer Rollback by default, Commit, or Cancel. Deferred
intents retain typed choices and cannot proceed after any prompt in the intent is
cancelled. Runtime disconnect remains defensive and closes matching workers even
if an App-level path fails.

## Execution Target Architecture

### Editor-local target

Every `ConsoleTab` owns a stable target:

```rust
struct ExecutionTarget {
    profile_id: Uuid,
    database: String,
    schema: Option<String>,
}
```

Connection generation is transient and is not part of persisted tab state. A
tab without a valid profile or target remains editable but cannot execute.

### Backend semantics

PostgreSQL database selection chooses the physical connection database and
therefore requires a new pool. Schema selection initializes safely quoted session
search-path behavior on every pool connection. It also drives completion's
default schema.

MySQL database and schema are the same namespace. The UI may retain the common
hierarchy, but their selected values remain identical. The database is set in
connect options; LazyDB does not issue `USE` before individual pooled queries.

SQLite database identifies the profile file, while schema identifies `main`,
`temp`, or a discovered attached alias. Initially, only aliases confirmed by the
active catalog may be selected. ATTACH and DETACH require catalog refresh before
their aliases become selectable.

### Safe tab activation

Activating a tab is asynchronous when its target differs from the active pool:

1. Check running queries and transaction lifecycle guards.
2. Record a pending tab activation without changing the active tab.
3. Compare the target with the runtime target.
4. If they match, complete activation immediately.
5. Otherwise start the existing safe connection-switch process.
6. Complete activation only after a matching connection identity succeeds.
7. On failure, retain the previous active tab and usable connection.

This prevents the UI from showing one target while execution still reaches the
previous target.

### Fail-closed execution

Every execution draft snapshots:

- console and document revision;
- exact `ExecutionTarget`;
- active connection identity;
- query and transaction generations;
- transaction mode and state;
- exact SQL scope.

Immediately before dispatch, the App validates that the tab target and Runtime
target match exactly. Any mismatch, pending connection, stale catalog, or changed
generation cancels execution with a visible reason. There is no fallback to the
global active connection.

### Target controls

The editor status area always shows:

```text
[profile] database.schema
```

It also shows READY, LINKING, OFFLINE, STALE, or FAILED when needed. Keyboard and
Ex entry points are:

```text
Space d c    Select connection profile
Space d d    Select database
Space d s    Select schema

:connection <profile-name>
:database <database>
:schema <schema>
```

The target area is clickable and opens a hierarchical selector. Changing profile
selects its default valid database and schema. Changing database resets schema to
a valid default. Target changes during active transactions pass through the
lifecycle gate.

Explorer "set as editor target" operations emit the same semantic actions. SQL
completion uses the active tab's target schema. Catalog and completion responses
carry profile, database, and catalog generation identity so stale results cannot
cross targets.

## Workspace Persistence

### Files

Workspace data is separate from connection profiles:

```text
~/.config/lazydb/
  connections.toml
  workspace.toml
  sql/
    <console-uuid>.sql
```

The versioned manifest stores active console and per-console metadata:

```toml
version = 1
active_console = "..."

[[consoles]]
id = "..."
name = "console_1"
sql_file = "sql/<uuid>.sql"
profile_id = "..."
database = "inventory"
schema = "public"
transaction_mode = "manual"
```

SQL text is stored in sidecar files rather than escaped TOML values. The
workspace never stores query results, output, popup state, selection, undo
history, connection generation, active transaction state, passwords, or
transient secrets.

### Save protocol

The App emits persistence commands after SQL changes, tab changes, active-tab
changes, target changes, and transaction preference changes. Runtime coalesces
ordinary saves with a 300-500 ms debounce and performs a forced flush on clean
shutdown.

Each save:

1. Writes changed SQL to private temporary files.
2. Flushes and atomically renames them.
3. Writes and atomically renames the new manifest after every referenced SQL file
   is durable.
4. Removes no-longer-referenced SQL files only after manifest success.

The manifest therefore never intentionally references SQL that was not written.
Persistence failure leaves the in-memory workspace intact and produces a visible
status error.

### Locking and recovery

Workspace persistence has one writer. If another LazyDB process holds the lock,
the application opens the workspace in explicit read-only persistence mode
rather than allowing last-writer-wins corruption.

Recovery behavior is fail-soft for content and fail-closed for execution:

- Missing manifest creates one empty console.
- Missing SQL sidecar restores an empty document with a warning.
- Missing profile preserves the SQL and marks the target MISSING PROFILE.
- Invalid or out-of-scope database/schema marks INVALID TARGET and blocks query
  execution until changed.
- MANUAL restores as MANUAL:IDLE.
- The restored active tab attempts safe automatic connection.
- Connection failure does not discard restored tabs or SQL.
- A newer unsupported manifest version is opened read-only and never overwritten.

## User Interface

The editor-local status line prioritizes Vim mode, execution target, and
transaction state:

```text
 NORMAL   [local-pg] inventory.public   TX MANUAL:ACTIVE   UTF-8
```

Narrow layouts progressively shorten labels but retain those three identities.
State is always textual and does not rely on color alone.

The target selector shows current selection and reasons for unavailable entries.
Connection changes display LINKING and disable execution. Escape cancels without
changing the target; Enter confirms only the visible selection.

The transaction menu is state-dependent:

- AUTO: Switch to Manual.
- MANUAL:IDLE: Switch to Auto.
- MANUAL:ACTIVE: Commit, Rollback, Switch to Auto.
- MANUAL:ABORTED: Rollback.
- OUTCOME_UNKNOWN: Verify/Reconnect.

Editor help documents Vim motions and objects, execution-target controls,
transaction controls, and their Ex equivalents.

## Error Handling

Previously silent failures become visible App status or output entries:

- unsupported editor action identifies the action class;
- execution target mismatch reports expected and actual target;
- failed auto-switch retains the prior tab and connection;
- persistence failure retains in-memory edits;
- workspace lock contention identifies read-only persistence mode;
- stale transaction commands are explicitly rejected;
- unknown commit or rollback outcome remains prominently unresolved;
- deleted profiles leave editable SQL with a blocked target.

## Delivery Stages

### Stage 1: Editor authority

- Introduce per-console modalkit machines and remove competing state.
- Route all editor input through one path.
- Drain the complete supported action queue.
- Finalize edits with revision and change effects.
- Render exact Visual selections and mode cursor styles.
- Add replacement cursor policies and fix completion undo.

### Stage 2: Transaction safety

- Fix typed exit choices and rollback defaults.
- Preserve exact transaction-control SQL scope.
- Add BEGIN readiness acknowledgement.
- Remove terminal registry entries and support repeated transactions.
- Validate complete runtime identity.
- Repair cancellation, shutdown, and unknown-outcome flows.
- Serialize query and transaction controls.
- Preserve structured transaction errors.

### Stage 3: Transaction controls

- Connect toggle and clear editor effects.
- Add status, menus, mouse targets, and Help content.
- Persist AUTO/MANUAL preference.

### Stage 4: Execution targets

- Add `ExecutionTarget` to consoles and execution drafts.
- Safely auto-switch tab targets.
- Add status selector, Ex commands, and Explorer actions.
- Make completion and catalog identity target-local.
- Implement backend-specific database/schema semantics.

### Stage 5: Workspace persistence

- Add versioned manifest and SQL sidecar store.
- Add atomic debounce saves and shutdown flush.
- Add single-writer locking and read-only mode.
- Restore tabs, active target, SQL, and mode preference.
- Handle missing profiles and invalid targets safely.

Stages 4 and 5 share a data model, but in-memory target correctness is verified
before disk restoration is enabled.

## Testing

### Editor

Tests use real key sequences such as `iabc<Esc>h`, `dw`, `ciw`, `daw`, `vip`,
`Ctrl-v`, `u`, `Ctrl-r`, character searches, and `gg/G`. They assert mode, text,
cursor, selection, revision, history, and change effects.

Full-pipeline tests cover Crossterm event through Keymap, App, EditorWorkspace,
and render snapshot. They verify tab isolation, pending text objects, Visual exit,
completion acceptance followed by typing and undo, and requested cursor style.

### Transactions

Tests cover keyboard and Ex controls, typed exit choices, repeatable SQLite manual
transactions, BEGIN failure, stale identity, unknown outcomes, cancellation,
shutdown, inactive-console lifecycle guards, PostgreSQL aborted state, and MySQL
implicit commit. PostgreSQL and MySQL integration tests remain conditional on
their existing environment variables.

### Execution targets

Tests cover successful and failed automatic tab switching, execution while
pending, stale completion/catalog responses, PostgreSQL pool recreation and
schema setup, MySQL namespace unification, SQLite alias validation, and target
changes guarded by active transactions.

### Persistence

Tests cover complete round trips, Unicode and large SQL, atomic-write failure,
manifest/sidecar consistency, orphan cleanup, missing files and profiles, invalid
targets, MANUAL-to-Idle restoration, lock contention, and shutdown flush.

## Acceptance Criteria

1. Normal-mode keys do not insert text and every documented Vim operation works
   through the production input path.
2. Normal, Insert, Replace, Visual, and prompt cursor styles are distinguishable.
3. Completion leaves the cursor after inserted text and is undoable.
4. Transaction state is visible; AUTO/MANUAL, commit, rollback, and recovery are
   discoverable and report truthful outcomes.
5. Each editor displays and owns one profile/database/schema target.
6. Tab activation safely auto-switches the one active pool and mismatches always
   block execution.
7. Tabs, SQL, target, active tab, and MANUAL preference restore automatically.
8. No active transaction is restored or implied after restart.
9. Formatting, Clippy, full tests, and configured database integration tests pass.
