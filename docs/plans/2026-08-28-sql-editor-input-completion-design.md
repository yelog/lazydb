# SQL Editor Input And Completion Design

**Status:** Approved

**Date:** 2026-08-28

## Summary

Make Insert/Replace mode `Ctrl+U` delete from the cursor to the current line
start, prevent completion acceptance from immediately reopening the same popup,
and present relation candidates with the object name as primary text and the
database/schema qualifier as muted detail.

## Design

Insert/Replace control keys are routed to the editor before global bindings. The
editor owns delete-to-line-start so Unicode boundaries, cursor placement, revision
tracking, effects, and undo remain coherent.

Completion acceptance is a programmatic edit. It applies normal editor effects
but suppresses completion scheduling for that one edit; the next user edit may
schedule normally. Existing revision validation rejects stale queued completion
work.

`CompletionCandidate` already separates `label`, `detail`, and `insert_text`.
Relation candidates use the unqualified object name as `label`, a deduplicated
`(database.schema)` qualifier as `detail`, and retain the existing context-aware
SQL insertion as `insert_text`. Columns and keywords keep their current detail
semantics.

## Verification

Cover single-line, multiline, Unicode, line-start, and undo behavior for Ctrl+U;
assert accepted completion stays closed until the next user edit; and verify
relation labels, qualifier details, insertion text, and muted popup rendering.
