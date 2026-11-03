# MongoDB relation document delete

Feature: the delete key on a MongoDB collection's relation tab marks rows for deletion and `Ctrl-S` commits them, bounded to each row's `_id`. Issue: gonzalobuassodev/lazydb#15.

## Specs

S1. The user reported, verbatim: "cada vez que quiero borrar un registro en mongodb", with the toast "Cannot update or delete existing rows without a primary key; no changes were saved". That toast must never be raised from a MongoDB relation row again.

S2. The delete key on the current row of a MongoDB collection's relation tab marks that row as pending deletion. It emits no command: nothing reaches the database until the save.

S3. With no document at the cursor, nothing is marked and nothing is emitted.

S4. `Ctrl-S` on a MongoDB relation with exactly one or more rows marked for deletion emits exactly one `Command::DeleteMongoRelationDocuments` carrying the tab id, tab generation, connection, namespace and every marked row's `_id`, and emits no `Command::RelationMutation`. The filter it becomes carries only `_id` and only those loaded values; an empty set is never emitted.

S5. With no row marked for deletion, `Ctrl-S` emits nothing and says nothing.

S6. A read-only MongoDB profile refuses the commit with the existing MongoDB read-only message, emits no command, and leaves the marked rows in place.

S7. A mark is never applied to the database twice: after a successful commit the page reloads and no row stays marked or dirty.

S8. A failed commit surfaces the error and keeps the page visible with the marks intact. A result whose tab id or generation is stale is ignored.

S9. A visual-mode range delete (`Action::RelationDeleteSelected`) marks every row in the range as pending deletion, and commits through the same `Ctrl-S` path.

S10. A row that is dirty with anything other than a deletion (`Updated`, or an insert draft) blocks the commit: a MongoDB relation never writes column updates or inserts through the grid, so the commit warns, emits nothing, and keeps every mark. This holds even when no row is marked for deletion, so an insert can never be a silent no-op.

S11. The browser tab's `d` delete (`Action::MongoDeleteSelected` → `Runtime::mutate_mongo_delete`) keeps working unchanged.

## Tasks

T1 | S1-S3, S9, S10 | `src/app.rs` — staged marks, range marks, commit gate, save branch | one commit
T2 | S4-S8 | `src/action.rs`, `src/runtime.rs`, `src/db/mongodb/` — plural command, bounded `deleteMany`, result handler | one commit
T3 | S1-S11 | `tests/mongodb_relation.rs`, `src/db/mongodb/mutation.rs` — one regression test per rule | one commit

## Log

L1. User report, verbatim: "cada vez que quiero borrar un registro en mongodb", with a screenshot of the toast "Cannot update or delete existing rows without a primary key; no changes were saved".

L2. Diagnosis: the relation tab reuses the SQL mutation path. A MongoDB collection reports no SQL columns (`src/model/mongodb_structure.rs:224-273`), so `metadata_fingerprint.primary_key` is empty (`src/db/mutation.rs:150-160`) and the keyless guard at `src/app.rs:29087-29091` refuses the save. The guard is pre-existing and SQL-scoped; the replace path already has a MongoDB-specific analogue (`Command::ReplaceMongoRelationDocument`) and delete has none.

L3. Decision: implement the full MongoDB delete path in the relation tab, mirroring replace. The stopgap (a Mongo-specific message pointing at the browser tab's `d`) and the no-code option were considered and rejected.

L4. Verified while reviewing T1-T3: visual mode is reachable in a MongoDB relation tab (`relation_visual_line` has no driver guard), and a visual range delete would have marked rows for a save that could never happen, silently. That hole was closed by refusing the range.

L5. Requirement change, user verbatim: "Ahora apretando dos veces la d se borra. Pero me gusta mas como funciona en prosgresql que marcas las filas y despues CTRL+S commiteas." The immediate delete of the first implementation is replaced by the staged flow SQL already uses: the delete key marks, `Ctrl-S` commits. S2, S4-S7, S9 and S10 are rewritten for it; the earlier refusal of the visual range (old S8) is superseded by S9, which marks the range instead.

L6. Design decisions taken with L5, not asked back to the user: one plural command and one bounded `delete_many({_id: {$in: [...]}})`, so the existing `Partial { affected, requested }` outcome finally carries meaning; `Updated` rows block the commit because a collection writes field edits through the document modal, not the grid; and a read-only profile is refused at commit time, matching where SQL refuses.

L7. Found while reviewing the staged rework: the refusal ordering put the empty-set early return before the dirty-row check, so a grid insert with nothing marked for deletion stayed silent — the same no-op class L4 closed. The dirty-row check now runs first, and S10 is widened to name insert drafts explicitly.

L10. Implemented T1-T3. Three points the specs left implicit, decided here: (a) S5 is honoured literally, so `relation_commit` keeps quiet for a MongoDB relation with no dirty rows instead of raising the shared "No pending relation changes" info the SQL grid raises; (b) while a deletion is pending, the commit refuses any row that is neither `Clean` nor `Deleted` (`InsertDraft`, `Inserted`, `Conflict`) with "MongoDB rows are edited in the document modal; the grid only commits deletions", because a grid insert shifts the row indexes the loaded `_id`s are read by; (c) the planner refuses a page whose document count no longer equals the row count, so a stale or shifted page can never pair a mark with another document's `_id`. `delete_one_filter` and `delete_document_by_id_detailed` are gone with the immediate delete; `delete_outcome_message` is still the single safety mapping.

L8. Independent verification of the staged rework returned SAFE TO COMMIT. Its advisories are recorded here rather than fixed: a second `Ctrl-S` before the first result lands repeats the same bounded ids (no widening, but a possible spurious "unknown outcome" toast after the success toast); a success on an inactive tab clears the marks without reloading it; `delete_outcome_message`'s `Confirmed { affected: 0 }` arm is unreachable from this path; and `bounded_delete_outcome` maps `affected > requested` to `Confirmed`, which `$in` cannot produce. The probes that pin the alignment guard, the shifted-grid refusal, the committed-delete-is-never-emitted-twice rule and the partial-outcome reporting were folded into `tests/mongodb_relation.rs`.

L9. The staged delete shipped in #16 (`develop` 8531981) and is temporarily absent from the tree after the `upstream/main` sync of 2026-10-31: upstream had independently extended the same field-editor work our #11 carried, and all 15 conflicts of that sync resolve to upstream. The implementation is not lost — it is in the history at 8531981 and in this document — and it lands again on the upstream base as a follow-up, which is why this spec stays in the tree while its code does not.

L11. Re-applied on the synced base (issue #19). Its independent verification returned SAFE TO COMMIT and found one defect worth fixing rather than recording: the partial/unknown message asked the user to "reload the page before retrying", but a refresh is refused while a mark is pending ("Commit or discard relation edits before changing pages"), so the instruction could not be followed. The message now names the discard first. The duplicate L7 in this log is renumbered L10. The remaining advisories still stand as recorded, and the count guard against a page that no longer matches the grid stays as defence in depth: no reachable UI path slips past the dirty-row refusal, but a catalog rebind can leave the page and the rows out of step.
