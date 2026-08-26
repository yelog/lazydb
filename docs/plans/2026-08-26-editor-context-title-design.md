# SQL Editor Context Title Design

**Status:** Approved

**Date:** 2026-08-26

## Summary

Transaction state and execution target belong to the active SQL Editor, not the
application header. LazyDB will remove them from the top-left Header and render
them as a right-aligned title on the SQL Editor border. The left title continues
to show SQL Editor and Vim mode. Help will document target and transaction
controls and explain their title indicators.

## Information Ownership

The global Header shows application and active-connection state only:

```text
LAZYDB  profile / connected database
ONLINE  QUERY IDLE
```

It no longer renders:

- transaction mode or state;
- editor execution target;
- editor profile UUID;
- editor database/schema.

The SQL Editor title owns console-local state:

```text
SQL EDITOR  NORMAL                         [moss_biz] moss_biz.tools  TX AUTO
```

Switching tabs updates this title from the newly active `ConsoleTab`.

## Block Titles

Use Ratatui's native multiple top titles:

- left-aligned title for `SQL EDITOR` and current editor mode;
- right-aligned title for execution target and transaction state.

The right title displays user-facing profile name, not `profile_id`:

```text
[profile-name] database.schema  TX state
```

Examples:

```text
[moss_biz] moss_biz.tools  TX AUTO
[prod-pg] inventory.public  TX MANUAL:ACTIVE
[sqlite] app.db.main  TX ABORTED
```

The editor content rectangle remains unchanged; no status row is inserted into
the text viewport.

## Context Projection

Add a pure UI projection that consumes:

- active `ConsoleTab::execution_target`;
- profile display names from `App::profiles`;
- `ConnectionState` and pending identity;
- transaction mode and state.

It returns sanitized display components:

```rust
struct EditorContextDisplay {
    profile: Option<String>,
    database: Option<String>,
    schema: Option<String>,
    target_state: TargetDisplayState,
    transaction: String,
}
```

The projection is shared by title rendering and Help descriptions. Internal UUIDs
never enter visible title strings.

Target states are textual and do not rely on color:

```text
READY
LINKING
OFFLINE
MISSING
INVALID
```

Transaction states retain the existing truthful labels:

```text
TX AUTO
TX MANUAL:IDLE
TX MANUAL:STARTING
TX MANUAL:ACTIVE
TX ABORTED
TX COMMITTING
TX ROLLING BACK
TX OUTCOME UNKNOWN
```

## Responsive Degradation

Transaction state has highest priority when the editor is narrow.

Wide:

```text
[moss_biz] moss_biz.tools  TX MANUAL:ACTIVE
```

Medium:

```text
moss_biz.tools  TX MANUAL:ACTIVE
```

Narrow:

```text
tools  TX MANUAL:ACTIVE
```

Smaller:

```text
TX MANUAL:ACTIVE
```

Extreme:

```text
TX ACTIVE
```

The degradation order is:

1. retain transaction state;
2. retain schema;
3. retain database;
4. retain profile display name last;
5. never display profile UUID.

The left editor title must also remain readable. If both titles cannot fit,
transaction wins on the right and the left mode label may use its existing short
form in compact layouts.

## Footer

The Footer keeps mode, concise shortcuts, and F1 Help. It does not repeat full
target or transaction state. It may include short discoverability hints:

```text
Space d target   Space tt tx
```

## Help

Editor Help adds an Execution Target group:

```text
Space d            choose connection target
:connection NAME   set connection profile
:database NAME     set execution database
:schema NAME       set default schema
```

It explains that the right title displays `[profile] database.schema` and that
execution is blocked when target and active connection differ.

The Transaction group includes:

```text
Space tt           toggle AUTO / MANUAL
Space tc           commit
Space tr           rollback
:tx auto/manual    set transaction mode
:commit            commit
:rollback          rollback
```

It explains that MANUAL preference restores as MANUAL:IDLE and no active
transaction is restored after restart.

The Help popup height is bounded by terminal area. Key target and transaction
entries must remain visible at 80x24; wording should be concise rather than
adding a scrolling subsystem solely for this change.

## Tests

UI render tests assert:

- Header omits transaction and execution target;
- SQL Editor right title contains profile/database/schema and transaction;
- no visible profile UUID;
- every transaction state maps correctly;
- wide, medium, narrow, and compact degradation;
- missing, offline, linking, and invalid target labels;
- tab activation changes the title context;
- Help contains target and transaction controls at 80x24.

Existing completion popup anchoring must continue to use the editor text viewport
and must not move because titles changed.

## Acceptance Criteria

1. Transaction appears only in the SQL Editor right title.
2. Current profile/database/schema appears beside transaction in the right title.
3. Header contains global connection/query information only.
4. Profile UUID is never visible.
5. Narrow layouts retain transaction before target details.
6. Help documents target indicators and controls.
7. SQL text viewport height does not change.
8. Existing UI, completion, mouse, and compact-layout tests remain green.
