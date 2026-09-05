# Workspace Save Queue and Failure Feedback Design

Status: partially implemented. The current worktree implements the bounded save
slot, revision-aware completion, retry retention, and flush-before-quit core.
Delete ordering, debounce, and full UI retry controls remain follow-ups. This document supersedes the queue
portion of T05 in the architecture optimization plan. No disk-format migration
or new dependency is required for the first implementation.

## Current Behavior and Risks

`Runtime::dispatch` spawns a task for each `PersistWorkspace` and `DeleteSqlFile`.
Tasks compete for `workspace_mutation`, discard errors, and are stored among
background tasks that `Runtime::shutdown` aborts. A mutex prevents simultaneous
writes but does not establish dispatch order between independently scheduled
tasks. Aborting a Tokio wrapper does not stop an already running blocking write.

`WorkspaceStore::save` writes SQL files before replacing the manifest. Individual
renames are atomic; the entire collection of files is not a transaction. A failed
save may already have changed some files. Retrying a complete snapshot must be
safe, but cannot undo those partial writes automatically.

## Ownership

- App owns durable-state revision and user-facing save/exit state.
- Runtime owns one save coordinator per WorkspaceStore and one active write job.
- The coordinator owns at most one in-flight snapshot and one latest pending snapshot.
- WorkspaceStore performs synchronous validation, SQL writes and manifest replacement.
- Blocking workers return results; they do not mutate App or decide to quit.
- The existing workspace lock must be acquired before loading and retained through
  the last worker completion. Verify the current startup lock path before wiring this in.

Use a single pending slot and wake notification, not an unbounded snapshot channel
and not one producer task per save. Replacing the slot drops the superseded
snapshot immediately. This bounds snapshot count, not bytes: SQL documents still
consume memory proportional to their combined size. Do not clone query results.

## Protocol

Proposed internal types (names may change during implementation):

```rust
struct SaveId {
    session: Uuid,
    revision: u64,
}

enum SaveEvent {
    Started { id: SaveId, attempt: u64 },
    Completed { id: SaveId, attempt: u64 },
    Failed { id: SaveId, attempt: u64, message: String },
}
```

`session` belongs to the writer instance. `revision` increases only when durable
state changes, never on viewport synchronization, query results, animations or
notifications. Use checked increments; exhaustion fails closed. Retry keeps the
same revision and increments attempt. Session/revision/attempt prevent late
events from clearing a newer failure. None of these counters need persistence.

Commands: OfferSnapshot(id, snapshot), RetryLatest, FlushThenClose(id), Resume.
The flush command carries a target revision; the coordinator must already own
that revision's immutable snapshot or reject the request rather than acknowledge
it prematurely. Command processing is serialized in Runtime dispatch order.

## Queue State Machine

| State | Input | Action | Next state |
| --- | --- | --- | --- |
| Idle | newer snapshot | install pending slot, arm 250ms debounce | Scheduled |
| Scheduled | newer snapshot | replace pending; do not extend beyond 2s from first dirty event | Scheduled |
| Scheduled | timer/flush | take pending, start exactly one blocking job | Writing |
| Writing | newer snapshot | replace pending only; never abort active write | Writing |
| Writing | success | record acknowledged revision; start latest pending if due | Idle/Scheduled/Writing |
| Writing | failure | preserve latest desired snapshot (failed snapshot if no newer one); report once | Failed |
| Failed | newer snapshot | replace retry snapshot; do not automatically retry on every keystroke | Failed |
| Failed | RetryLatest | start latest desired snapshot with new attempt | Writing |
| Any open state | FlushThenClose(target) | disable debounce; latch close target; retain event processing | Flushing |
| Flushing | acknowledged revision >= target, no active write | close coordinator and release lock | Closed |
| Flushing | write failure | keep snapshot and lock; expose retry/stay/discard choices | FailedClosing |
| FailedClosing | retry | retry latest final snapshot | Flushing |
| FailedClosing | stay | clear close latch; resume application editing | Failed |

Represent queue mechanics (active job/pending snapshot/close latch) separately
from the UI enum so newer dirty edits can coexist with an older write in flight.
Do not turn every combination into a new enum variant. Worker panic/channel loss
is a failure, never an acknowledgement; retain a retryable snapshot until success.

## Deletion Ordering

Prefer removing the separate user-triggered DeleteSqlFile command. A snapshot
already contains the complete desired manifest, so deletion becomes cleanup
after a successfully persisted manifest no longer references the console UUID.
This removes a save/delete queue ordering race instead of encoding more barriers.

For the first implementation, defer cleanup to a separate, idle-only coordinator
step. Track only console IDs that were known in the previous acknowledged
manifest; never glob-delete unrelated files. Before cleanup, check both the
acknowledged manifest and latest desired snapshot. Skip any ID referenced by
either. Console UUIDs must not be reused for unrelated new documents.

Cleanup runs serially with writes. Missing files are success. Permission/I/O
failure emits a cleanup warning and retains bounded-by-known-documents cleanup
state; it must not relabel an otherwise durable manifest as unsaved. Cleanup can
be deferred until the next successful save, and must not block ordinary exit.

If separate DeleteSqlFile must remain for external callers, treat it as a strict
FIFO barrier and provide bounded admission explicitly; do not merge snapshots
across it. Do not ship both protocols without a demonstrated consumer need.

## UI State Machine

| UI state | Meaning | Feedback |
| --- | --- | --- |
| Clean | acknowledged revision equals current durable revision | no toast |
| Dirty | current revision is not acknowledged | quiet unsaved indicator |
| Saving | write active; newer edits may still be dirty | quiet saving indicator |
| Failed | last attempt failed and latest revision remains unsaved | persistent unsaved indicator and error notification |
| Closing | final snapshot flushing; editing frozen | saving-before-exit message |
| CloseFailed | final save failed | Retry / Stay / Exit Without Saving |

Older success may advance acknowledged revision but cannot make the UI Clean
while a newer durable revision exists. An older failure is retained in history
but cannot replace a newer successful status. Ignore events from another writer
session or obsolete attempt. Transient notification expiry does not clear Failed.

Failure messages are sanitized and never include SQL text or credentials.
Use notification source Workspace (or an explicitly documented existing source).
Deduplicate repeated failure notifications by attempt/error category. Successful
recovery clears the failure indicator and may show one recovery notification;
ordinary autosave never spams success toasts.

Retry persists the latest snapshot, not an obsolete failed version. No automatic
retry loop in v1: disk-full, permissions and invalid paths are usually persistent.

## Quit and Restart

1. Resolve existing transaction exit prompts first, without changing their safety rules.
2. Freeze durable edits, create/offer the final snapshot and enter Closing.
3. Keep the event loop alive until its flush result arrives. Do not set should_quit yet.
4. On success, shut down other runtime resources, await writer join, restore the terminal,
   then exit or launch the requested replacement binary.
5. On failure, show Retry / Stay / Exit Without Saving. Discard requires explicit
   confirmation and means abandoning pending state, not rolling back files already written.
6. A slow-save timeout offers waiting or staying; it must not abort an active
   spawn_blocking job or release its lock while the write still runs. Restart is
   prohibited until the old writer is quiescent. Repeated quit/Ctrl-C requests are idempotent.

The shutdown fallback also awaits a live writer and reports errors through the
returned run result after terminal restoration. Do not rely only on Actions that
the terminated event loop can no longer receive. External kill/power loss remains
outside graceful-exit guarantees; multi-file crash atomicity needs a later design.

## Incremental Writes

The current content comparison skips unchanged SQL writes but rereads all files.
The follow-up coordinator should use per-document revision plus editor-session
identity against the last successfully persisted versions. Reopening a document
must not accidentally reuse an old revision token. Advance acknowledged versions
only after the whole save succeeds; partial failure can safely rewrite unchanged
files during retry. Preserve atomic file replacement and existing format readers.

## Implementation Slices

1. Add `src/model/workspace_save.rs`: pure status transitions and stale-event tests;
   wire durable revision changes in `src/app.rs` without yet switching the writer.
2. Add `src/runtime/workspace.rs`: one pending slot, one blocking writer, debounce,
   failure retention and explicit flush. Inject storage for deterministic tests.
3. Wire `src/action.rs`, Runtime dispatch and WorkspaceStore; replace background-task
   persistence and remove old mutex/task paths only after all call sites migrate.
4. Add safe post-manifest deletion cleanup and test save/delete/reopen sequences.
5. Wire notifications and quit/restart gating. Update help/config docs if any keys
   or user settings change. Timer defaults are internal in v1.
6. Validate in temporary directories, then run complete local and isolated DB tests.

## Required Tests

- Offer 1/2/3 before timer: only revision 3 writes. Offer 4/5 while 3 writes:
  exactly 3 then 5; no overlap; queue holds at most two snapshots.
- Failure after one SQL rename: no false acknowledgement; latest snapshot remains
  retryable and can replace the partially persisted state.
- Older success/failure, wrong session, duplicate completion and prior retry attempt
  cannot overwrite newer status.
- Repeated edits while failed do not cause a retry storm or unbounded queue growth.
- Delete then reopen before cleanup preserves the file; deletion failure warns but
  does not mark a saved manifest dirty; unrelated files remain untouched.
- Quit with pending write waits; save failure offers choices; Stay resumes;
  discard does not pretend to restore pre-save files; restart waits for writer join.
- Worker panic, read-only directory, disk-write failure and manifest rename failure
  are distinct observable failures. Use fake I/O for deterministic injection.
- Timer tests use paused Tokio time if the existing feature set supports it, or
  an injected clock; never arbitrary sleeps. Count actual write calls, not mtime alone.

Acceptance: no silently ignored persistence failure, no active writer aborted at
graceful exit, no obsolete snapshot overwrites a newer one, and saved status always
means the latest durable state was acknowledged. No claim of cross-file atomicity.
