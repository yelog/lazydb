# SQL Editor, Vim, and Transaction Design

- Status: Approved
- Date: 2026-08-25
- Scope: SQL console editing, language assistance, safe execution, and manual
  transactions
- Target databases: PostgreSQL, MySQL, and SQLite

## 1. Goals and Decisions

This increment turns the M0 SQL console into a practical database editor while
preserving LazyDB's reducer and database boundaries. It provides:

- A useful Vim core: Normal, Insert, Replace, Visual Char/Line/Block, counts,
  common motions, operators, text objects, undo/redo, dot repeat, registers,
  system clipboard, search, and substitution.
- SQL highlighting, formatting, and catalog-backed semantic completion.
- Selection-first or current-statement execution with immutable previews and
  risk-based confirmation.
- Per-console AUTO and MANUAL transaction modes with real pinned connections,
  explicit commit/rollback, and truthful failure states.
- Contextual status, overlays, Ex commands, and discoverable shortcuts.

The approved product choices are:

- Single, syntactically read-only statements execute directly by default.
  DML, DDL, transaction control, unknown SQL, and multi-statement drafts require
  confirmation. A setting can require confirmation for every statement.
- The main run action uses Visual selection when present, otherwise the
  statement under the cursor. It never silently falls back to the whole buffer.
- MANUAL mode starts lazily on first execution. Commit or rollback ends the
  current transaction but leaves the console in MANUAL mode.
- Closing a console, changing connections, leaving MANUAL mode, or quitting with
  an active transaction prompts for Commit, Rollback, or Cancel.
- Completion combines context-aware automatic triggering with explicit
  `Ctrl-Space` triggering.

## 2. Architecture

The existing one-way flow remains authoritative:

```text
Key / Mouse / Tick / DB Event
          -> Action
          -> App::update
          -> Command
          -> Runtime
          -> Result Action
          -> Render
```

Only `App::update` mutates application state. The editor, SQL services, and
transaction implementation fit into that flow rather than introducing direct UI
or event-loop mutation.

### 2.1 Editor Ownership

`App` owns a global `EditorWorkspace`. It privately contains:

- A `modalkit::Store` with buffers, cursors, registers, and completion state.
- A Vim binding machine and command machine.
- One editor session per SQL console, keyed by the console UUID.
- Shared registers and clipboard state so yank/paste works across consoles.

`ConsoleTab` stores a stable editor session identifier plus query, completion,
and transaction presentation state. It does not duplicate editor text.

No `modalkit` type escapes the editor module. The rest of LazyDB uses a small
domain API for text, cursor, selection, mode, range replacement, key handling,
and immutable render snapshots. This isolates pre-1.0 dependency churn.

Use exact `modalkit` version `0.0.25` with its cross-platform `clipboard`
feature. Its declared MSRV is Rust 1.75, so LazyDB remains on Rust 1.94. Do not
use `modalkit-ratatui`, which targets a different Ratatui version. Remove the
unused `ratatui-textarea` dependency.

### 2.2 Input Boundary

The top-level keymap handles overlays, focus-level input, and unambiguous global
keys. Editor input becomes `Action::EditorKey(KeyEvent)`. `App::update` sends the
key to `EditorWorkspace`, drains the resulting editor actions, and maps custom
application actions to LazyDB commands.

Leader sequences in the editor are registered as `modalkit` application actions.
This avoids two independent pending-key state machines.

Input priority is:

1. Blocking overlay or open completion popup.
2. Insert/Replace mode editor keys.
3. Focus-level and mode-appropriate LazyDB global keys.
4. Normal/Visual mode Vim bindings.

This specifically means `Ctrl-W` deletes the previous word in Insert mode and
starts pane navigation only in Normal mode.

### 2.3 SQL and Runtime Boundaries

A pure SQL language service consumes editor text, cursor/selection, database
dialect, and a catalog snapshot. It returns statement ranges, token spans, risk
classification, formatting edits, and completion candidates. The editor buffer
is always the only text source of truth.

`App` stores only displayable transaction mode and state. `Runtime` owns a
transaction registry keyed by console UUID. SQLx pool connections and
transactions never enter the UI model.

The UI consumes an immutable `EditorRenderSnapshot` and application overlay
models. It renders them with LazyDB's Ratatui 0.30 theme and layout rather than a
third-party widget.

## 3. Vim Editing

### 3.1 Compatibility Target

The first-version acceptance target includes:

- Normal, Insert, Replace, Visual Char, Visual Line, and Visual Block modes.
- Counts and common motions, including word, line, buffer, character-find,
  matching-delimiter, paragraph, and page motions.
- Delete, change, yank, indent, dedent, case, and paste operations composed with
  motions and text objects.
- Undo/redo with Vim-style insert-session grouping and dot repeat.
- Unnamed, named, and system clipboard registers.
- `/`, `?`, `n`, and `N` search behavior.
- `:s` and `:%s` substitution over supported ranges.
- Insert-mode `Ctrl-W` to delete the previous word, `Ctrl-U` to delete to the
  start of the line, and `Ctrl-H` as Backspace.

Macros, marks, folds, multi-window Vim UI, complete Ex compatibility, and exact
Vim regex magic semantics are not first-version acceptance requirements even if
the selected engine exposes some of them.

### 3.2 Editor Effects and Revisions

Each editor key is processed inside the reducer. `EditorWorkspace` emits domain
effects such as:

- Buffer or selection changed.
- Run current scope or full buffer.
- Format current scope.
- Trigger, navigate, accept, or dismiss completion.
- Toggle transaction mode, commit, or rollback.
- Close the current console.

A text edit increments the console's `document_revision`, invalidates stale
completion, and invalidates derived SQL analysis. Formatting, completion
acceptance, and one substitution command each create one undo checkpoint.

### 3.3 Substitution

`modalkit 0.0.25` provides the Ex command line and range parser, but its built-in
substitute command is explicitly unimplemented. LazyDB therefore registers its
own `:substitute`/`:s` application command.

The handler reuses `modalkit` command and range parsing, then applies a pure
`SubstitutionSpec` to the Ropey buffer. It supports:

- Current-line, `%`, Visual, and numeric line ranges.
- Non-alphanumeric delimiters and escaped delimiters.
- `g`, `i`, `I`, and interactive `c` flags.
- `&`, capture-group replacement, and reuse of the previous non-empty pattern.
- A single undo checkpoint for the complete accepted substitution.

Matching uses Rust regex semantics. Unsupported Vim-specific magic constructs
produce a clear command-line error rather than a partial replacement.

## 4. SQL Language Services

### 4.1 Dialect and Statement Ranges

Select the `sqlparser` dialect from the active server: PostgreSQL, MySQL, or
SQLite, with a conservative generic fallback while disconnected.

Statement boundaries use a dialect-aware lexical scanner instead of requiring
the complete buffer to parse. The scanner tracks line and block comments,
single-quoted strings, double-quoted identifiers, MySQL backticks, SQLite bracket
identifiers, and PostgreSQL dollar-quoted strings. A semicolon is a boundary only
outside those constructs.

`sqlparser` then parses the candidate statement when possible. Parse failure does
not prevent highlighting, range selection, or confirmation; it makes risk
classification `Unknown`.

Scope precedence for execution and formatting is:

1. The exact Visual selection.
2. The non-blank statement containing the cursor.
3. No scope, reported as a status message.

Whole-buffer execution is a separate explicit action. The first version does not
promise current-statement recognition inside every vendor procedural-language or
custom MySQL `DELIMITER` construct; users can select the exact text or use the
explicit full-buffer action, which is classified conservatively.

This scoped implementation uses a tested scanner plus `sqlparser` rather than
adding Tree-sitter immediately. It supersedes the tentative Tree-sitter choice in
the original product design for this increment. Tree-sitter remains an option if
the dialect corpus demonstrates a concrete gap that cannot be handled cleanly.

### 4.2 Highlighting

Tokenizer output produces spans for keywords, identifiers, strings, numbers,
comments, operators, and parameters. Catalog lookup adds semantic styles for
known tables, views, columns, functions, and procedures.

Incomplete or invalid SQL falls back locally to plain text. Highlighting never
blocks input or opens an error overlay. Derived analysis is cached by console
UUID, document revision, dialect, and catalog generation so render does not
reparse unchanged text.

### 4.3 Formatting

Formatting uses the existing `sqlformat` dependency over the selected scope. The
default is stable indentation with uppercased keywords. The formatted range
replaces the original range as one editor transaction, keeps the cursor near the
range start, and can be reverted with one `u`.

### 4.4 Semantic Completion

Completion reads only the cached catalog; typing never queries the database.
Trigger rules are:

- Immediately after `.`.
- About 120 ms after identifier input stops.
- Immediately on `Ctrl-Space`.

Context ranking prefers tables/views after FROM or JOIN, columns after a
qualifier, routines in function positions, and aliases in the current statement.
Remaining candidates mix suitable keywords and catalog symbols. Ranking uses
context fit, prefix fit, current schema, then stable lexical order.

When the popup is open, `Ctrl-N/P` moves, Enter accepts, and Escape closes it.
When closed, those keys retain their editor meanings. Acceptance replaces only
the active token and is one undoable edit.

## 5. Safe Execution

### 5.1 Immutable Execution Draft

The run action first creates an immutable `ExecutionDraft` containing:

- Console UUID, query generation, connection generation, and document revision.
- Scope kind, source range, and exact SQL snapshot.
- Dialect, statement count, transaction mode/state, and risk classification.

Confirmation executes this snapshot, not newly read editor text. Before runtime
dispatch, the reducer verifies that the tab, document revision, and connection
generation still match. A stale draft is discarded and must be recreated.

### 5.2 Classification

Parsed statements are classified as `ReadOnly`, `Dml`, `Ddl`,
`TransactionControl`, or `Unknown`. Classification recursively inspects wrappers
such as WITH and EXPLAIN. Parser failures, unsupported statements, CALL, and
dynamic SQL are `Unknown`.

This is a syntax risk signal, not proof that a SELECT cannot invoke a function
with side effects. Database permissions and a read-only connection profile remain
the enforcement boundary.

### 5.3 Confirmation Policy

Default behavior is:

- One known read-only statement executes directly.
- DML, DDL, transaction control, unknown SQL, and every multi-statement draft
  require confirmation.
- Explicit full-buffer execution always requires confirmation.
- A setting can require confirmation for all statements.

The overlay shows scope and line range, statement count, risk, database/dialect,
transaction state, and the complete scrollable SQL. Cancel is the default action.
MySQL DDL in MANUAL mode includes an implicit-commit warning.

A read-only connection profile remains enforced by each database adapter and
cannot be bypassed by accepting a UI confirmation.

One console can have only one active query. Different AUTO consoles can execute
concurrently.

## 6. Transactions

### 6.1 Model and Runtime Ownership

Each console has `TransactionMode::{Auto, Manual}` and a display state:

```text
Idle -> Starting -> Active -> Committing -> Idle
                         \-> RollingBack -> Idle
                         \-> Aborted -> RollingBack -> Idle
                         \-> OutcomeUnknown
```

AUTO executes against the pool and relies on database autocommit. MANUAL Idle
starts a transaction lazily on the first execution. Commit and rollback return
to MANUAL Idle.

Runtime maintains one serial transaction worker per active MANUAL console. The
worker owns a concrete SQLx transaction enum for PostgreSQL, MySQL, or SQLite.
All statements and transaction operations for that console pass through the
worker and therefore the same physical connection.

Database result collection is shared between pool and transaction executors; the
SQLx concrete types remain inside adapter modules.

### 6.2 Explicit Transaction SQL

A single `BEGIN` or `START TRANSACTION` in AUTO, after confirmation, changes the
console to MANUAL and starts a transaction. In MANUAL Idle it starts immediately.
It is rejected if a transaction is already active.

A single `COMMIT` or `ROLLBACK` maps to the corresponding worker command rather
than being sent to a random pool connection. SAVEPOINT-related statements require
an active MANUAL transaction.

The first version rejects a draft that mixes transaction-control statements with
other statements. It tells the user to enter MANUAL mode and execute the
statements separately. This prevents the UI state machine from diverging from the
database session.

### 6.3 Errors and Cancellation

PostgreSQL statement errors move the transaction to Aborted and only rollback is
allowed. MySQL and SQLite keep Active after ordinary database or constraint
errors. A network, protocol, or lost-connection error drops the worker and relies
on SQLx transaction ownership to start rollback rather than returning a
potentially dirty connection as active.

Cancelling a MANUAL query terminates its worker and rolls back the complete
current transaction. The UI warns that all earlier uncommitted work in that
transaction will also be lost.

MySQL DDL can implicitly commit before and after execution. Once such DDL is sent,
the worker treats the transaction as ended, releases the session, returns to
MANUAL Idle, and reports that an implicit commit may have occurred whether the
DDL itself succeeded or failed.

A lost commit acknowledgement produces `OutcomeUnknown`. LazyDB closes the
session, never retries commit, never reports rollback, and blocks further
mutating work in that console until the user reconnects or explicitly clears the
state after verification. Rollback acknowledgement failure is reported with the
same honesty.

All transaction and query events include console, query, and connection
generations so late worker results cannot mutate a newer session.

### 6.4 Leaving a Transaction

Switching MANUAL to AUTO, closing a console, replacing the active connection, or
quitting with an active or aborted transaction suspends the requested operation.
Transactions are resolved one console at a time with Commit, Rollback, or Cancel;
Rollback has default focus. A running query must finish or be cancelled first.

Emergency runtime shutdown drops all workers and starts best-effort rollback. It
never intentionally commits an unresolved transaction.

## 7. UI and Commands

The editor status line shows Vim mode, line/column, SQL dialect, current scope,
and one of:

- `TX AUTO`
- `TX MANUAL:IDLE`
- `TX MANUAL:ACTIVE`
- `TX ABORTED`
- `TX COMMITTING`
- `TX ROLLING BACK`
- `TX OUTCOME UNKNOWN`

Warnings and failures use the existing theme's warning/error styles.

### 7.1 Default Keys

| Key | Action |
| --- | --- |
| `F5`, `<leader>r` | Run Visual selection or current statement |
| `Shift-F5`, `<leader>R` | Preview and run the complete buffer |
| `<leader>f` | Format Visual selection or current statement |
| `Ctrl-Space` | Trigger completion |
| `Ctrl-N/P` | Move through an open completion popup |
| `<leader>tt` | Toggle AUTO/MANUAL transaction mode |
| `<leader>tc` | Commit the active MANUAL transaction |
| `<leader>tr` | Roll back the active MANUAL transaction |
| `Ctrl-C` | Cancel the active query |
| `Ctrl-W h/j/k/l` | Move pane focus in Normal mode |
| `F1`, `<leader>?` | Open editor help |

In Editor Normal mode, `?` retains Vim backward-search semantics. In Explorer and
Results, `?` can continue to open contextual help. `Q` requests application exit
only in Normal mode or outside the editor, so Insert mode can enter an uppercase
Q normally.

### 7.2 Ex Commands

LazyDB registers:

- `:run`
- `:runall`
- `:format`
- `:tx auto` and `:tx manual`
- `:commit`
- `:rollback`
- `:q`

These produce the same application actions as shortcuts. They do not create a
second execution path.

### 7.3 Overlays and Completion

The completion popup is cursor-anchored, clamped to the viewport, and displays up
to about ten symbol, kind, and detail rows.

The execution overlay has Execute and Cancel, with Cancel focused initially. The
transaction-exit overlay has Commit, Rollback, and Cancel, with Rollback focused.
Unavailable operations are disabled with an explanation.

The contextual help overlay groups editor bindings into Navigation, Editing,
SQL, Completion, Transaction, and Tabs/Windows. It lists only implemented keys.
The command/search line temporarily replaces ordinary status text at the bottom.

## 8. Error and Security Rules

- SQL analysis failures degrade to `Unknown` or plain highlighting; they do not
  stop editing.
- Database failures retain the SQL snapshot and append sanitized detail to the
  console Output view.
- Database text continues through terminal-control sanitization before rendering.
- Confirmation is not an authorization mechanism and never overrides read-only
  adapter configuration.
- Unknown transaction outcomes are explicit and are never automatically retried.
- Empty execution scope, stale draft, unsupported Ex syntax, and unavailable
  transaction actions produce actionable status messages rather than panics.

## 9. Testing and Acceptance

### 9.1 Editor Contract Tests

Table-driven key-sequence tests cover mode changes, counts, operator-motion and
text-object composition, Visual modes, history, dot repeat, cross-console
registers, search, and Insert-mode `Ctrl-W`/`Ctrl-U`.

Unicode cases include CJK, emoji, combining characters, and double-width glyphs.
Tests assert cursor and selection positions, edits, and rendering rather than
only checking for absence of panics.

Substitution tests cover current, `%`, Visual, and numeric ranges; escaped and
custom delimiters; `g/i/I/c`; capture groups; `&`; previous-pattern reuse; no
matches; invalid regex; and one-step undo.

### 9.2 SQL and Reducer Tests

Pure SQL tests cover semicolons inside every supported quote/comment form,
cursor boundaries, selection precedence, CTE DML, multi-statement maximum risk,
parser fallback, three-dialect highlighting, aliases, qualified-column
completion, and catalog-generation invalidation.

Reducer tests cover confirmation policy, immutable and stale drafts, read-only
enforcement, query concurrency, mode-sensitive key priority, every transaction
transition, and close/quit resolution.

### 9.3 Runtime and Database Tests

A fake transaction session exercises begin, execute, commit, rollback, cancel,
connection loss, outcome unknown, and stale worker events deterministically.

Temporary SQLite integration tests are mandatory and prove connection pinning,
commit, rollback, errors, cancellation rollback, and connection release.
Existing environment-gated PostgreSQL and MySQL tests add pinned-session behavior,
PostgreSQL aborted transactions, and MySQL implicit DDL commit. If those service
URLs are absent, tests report the skip and the final result must not claim those
integrations were run.

### 9.4 UI and Performance Tests

Ratatui `TestBackend` tests cover 120x36, 80x24, and the smallest usable layout.
They verify mode/TX status, Unicode selection, highlighting, completion clipping,
execution and transaction overlays, and contextual help while preserving current
layout regressions.

A 10,000-line SQL fixture verifies that input invalidates only derived analysis,
render performs no database work, and completion remains catalog-local. New
background threads are not added without a measured need.

### 9.5 Completion Gate

The increment is complete when all existing and new tests pass along with:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Implementation also updates `README.md`, `docs/architecture.md`, and
`docs/keybindings.md` to match the shipped behavior.

## 10. Explicit Non-Goals

- Complete Vim or Ex compatibility.
- Exact Vim regex magic syntax.
- Full procedural-language parsing for every database extension.
- Database requests on each completion keystroke.
- Continuing a MANUAL transaction on a different physical connection.
- Automatic retry after an unknown commit result.
- Silent whole-buffer execution when current-statement detection fails.
