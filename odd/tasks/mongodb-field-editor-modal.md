# MongoDB field editor modal

Feature: every field of a MongoDB document opens in its own modal over the document modal, sized to the value; an array or embedded document opens there as pretty-printed Canonical EJSON. Issue: yelog/lazydb#25.

## Specs

S1. "la manera de editar en mongodb un array es sumamente incomodo. quiero que mejores esto quizas abriendo un nuevo modal para editar el campo completo en este modal. El array u objeto tiene que estar mejor formateado."

S2. (S1, "abriendo un nuevo modal para editar el campo completo") In the FIELDS list of the document modal, `Enter` on an array or embedded-document field opens a dedicated editor over the modal that edits that whole field. The document modal's header identifying the collection and `_id` stays visible.

S3. (S1, "El array u objeto tiene que estar mejor formateado") The field editor shows the value as indented, multi-line Canonical Extended JSON, scrolled with the cursor, and `Ctrl-F` reformats it. The field list shows a shape marker for those fields (`[3 items]`, `{5 fields}`) instead of clipped JSON, matching what the data grid already shows for containers.

S4. Constraint (existing rules, unchanged): applying a field only rewrites the document text; saving stays one `replaceOne` bounded to the original `_id`; `_id` is never editable; scalar fields keep the single-line editor and its `Enter`-applies behavior.

S5. Constraint: `src/db/value.rs`, `src/db/mongodb/*` and the write path are not modified; the feature is editor-side only.

S6. "el modal es demasiado grande, me gustaria mas pequeño." The field modal is sized to the value it edits (its longest line and its line count, plus borders, header and footer) instead of covering the relation area, and it never grows past the document modal it opens over.

S7. "el resto de los campos tambien quiero que se abran en un modal." Every editable field of the FIELDS list opens in the field modal, scalars included. A scalar opens it as a one-line value: `Enter` or `Ctrl-S` validates and applies to the document draft and returns to the list; `Esc` discards the current field edit and returns to the list. `Up`/`Down` validates and applies to the document draft, then opens the previous or next editable field, skipping `_id` and stopping at the ends. Invalid input keeps the current field editor and its text open with an error and leaves the document draft unchanged. Field apply and navigation never send a database write.

S8. Constraint (replaces part of S4): the field list no longer renders an inline editor row; `_id` stays listed and never editable, and saving still rewrites only the document text and stays one `replaceOne` bounded to the original `_id`.

S9. "Quiero que siempre el modal tenga margenes por arriba y por abajo del texto que contiene, para que no quede todo amontonado" The field modal keeps one blank row between its document header and the value, and one blank row between the value and its footer, for every field kind and whether or not a refusal is showing.

S10. "Ahora en campo date no lo formatea correctamente." A BSON `date` reads and edits as ISO-8601 UTC (`2026-11-08T00:00:00Z`, with milliseconds only when it has them) in the field list and in the field modal, never as canonical EJSON. Applying accepts that form, `2026-11-08 15:30[:15[.250]]`, `2026-11-08` and canonical or relaxed EJSON, the forms without an offset as UTC; the document text keeps the canonical EJSON the write path stores.

S11. Contextual Help and footer rows distinguish the FIELDS list, single-line field modal, multi-line field modal and whole-document EJSON view, without SQL NULL/DEFAULT controls. FIELDS uses `Ctrl-S` to save the document and `Tab` to switch to EJSON. A multi-line field uses `Enter` for newline, `Ctrl-S` to validate and apply to the document draft, `Ctrl-F` to format, `Up`/`Down` to move the cursor and `Esc` to discard that field edit. Whole-document EJSON uses `Ctrl-S` to save, `Ctrl-F` to format, `Enter` for newline, `Up`/`Down` to move the cursor, `Tab` to return to FIELDS and `Esc` to cancel document editing.

## Tasks

- T1 (S2, S3) — Field edit holds either a single-line or a multi-line input, containers open on indented EJSON, and applying parses with the field's BSON type. Route: inline. Commit: `feat(mongodb): edit every document field in its own modal`.
- T2 (S2) — Keys for the field editor (Enter newline, Ctrl-S apply, Ctrl-F format, Esc back, Up/Down cursor) without changing the scalar editor or the EJSON view. Route: inline. Commit: `feat(mongodb): edit every document field in its own modal`.
- T3 (S2, S3) — Render the field editor as a popup over the document modal and show shape markers in the field list. Route: inline. Commit: `feat(mongodb): edit every document field in its own modal`.
- T4 (S2, S3) — Document the modal's keys in `docs/keybindings.md`. Route: inline. Commit: `docs(keybindings): document every field modal of the mongodb document view`.
- T5 (S6) — Size the field modal to its content and keep it inside the document modal. Route: inline. Commit: `feat(mongodb): edit every document field in its own modal`.
- T6 (S7, S8) — Open scalar fields in the same modal and drop the inline editor row. Route: inline. Commit: `feat(mongodb): edit every document field in its own modal`.
- T7 (S6, S7, S8) — Cover both with tests and update `docs/keybindings.md`. Route: inline. Commit: `feat(mongodb): edit every document field in its own modal`.
- T8 (S9) — Blank margin row above and below the value inside the field modal. Route: inline.
- T9 (S10) — A BSON date shows and edits as ISO-8601 UTC. Route: inline.

## Log

L1. "la manera de editar en mongodb un array es sumamente incomodo. quiero que mejores esto quizas abriendo un nuevo modal para editar el campo completo en este modal. El array u objeto tiene que estar mejor formateado."

L2. Filed as lansisDev/lazydb#6 (`enhancement`) in the contributing fork before the first line of code, and filed again as yelog/lazydb#25 when the work went upstream, which is the issue this document tracks.

L3. Decision (scope, superseded by L9): the multi-line modal opens for `Bson::Array` and `Bson::EmbeddedDocument` only. Other `Ejson`-kinded scalars (`date`, `decimal128`, `regex`, `binary`, `timestamp`) keep the single-line input, because the user named arrays and objects and their text is a short canonical token.

L4. Decision (where the modal draws, superseded by L8): the field editor is its own popup, not a popup nested inside the 88x24 document modal, so an array gets real room. It repeats the document header (`collection · _id`) so the bounded write target stays visible, and it reuses `render_json_editor` (cursor, scrolling, syntax colours) with its own footer.

L5. Decision (list rows): a container row shows `[n items]` / `{n fields}` instead of clipped canonical EJSON, matching the shape marker the data grid already uses for the same values. `DocumentField::value` keeps the compact EJSON, because it is the text a single-line editor starts from and the tests pin it.

L6. Compromise (review workload): the behavior is about 800 authored lines, above the ~400-line work-unit target. The modal spans model, keymap, app, UI and tests as one unit, so a per-file split would have produced intermediate states that do not compile; the repository already recorded that trade-off as the lesser evil (see `odd/tasks/mongodb-relation-tab.md` L15). It reaches upstream as three chained pull requests — the modal with its margins, the date slice, and the keybindings docs — so each one stays reviewable on its own.

L7. "ahora esta bien... dos cosas... el modal es demasiado grande, me gustaria mas pequeño. Y el resto de los campos tambien quiero que se abran en un modal."

L8. Decision (size, answers L7): the popup is sized by the widest of its own parts — the value, the title on the border and the footer — capped at 72 columns, and by its line count plus borders, capped at 20 rows, so it never covers the document modal. `DocumentFieldInput::line_count`, `longest_line_columns` and `modal_body_height` exist for that.

L9. Decision (scope, replaces L3 and half of S4): every editable field opens the modal. A scalar opens it on one line, where `Enter`/`Ctrl-S` validates and applies to the document draft and returns to the list, and `Esc` discards the current field edit and returns to the list. `Up`/`Down` validates and applies to the draft, then opens the adjacent editable field, skipping `_id` and stopping at the ends. Invalid input keeps the current editor and its text open with an error and leaves the document draft unchanged. These actions never send a database write. The field list no longer renders an inline editor row.

L10. Verification. Focused: 44 tests in `tests/mongodb_relation.rs`, 167 in `tests/keymap.rs`. On the fork branch the full suite (with `LAZYDB_DUCKDB_LIB_DIR=target/duckdb-runtime/aarch64-apple-darwin`) reached 3446 passed, 0 failed, exit 0. `cargo fmt --check` clean; clippy reports nothing in the touched files (four warnings remain in `src/db/clickhouse/definition.rs`, `src/ui/footer.rs`, `src/value_preview/edit.rs` and `src/model/mongodb_browser.rs`, all pre-existing and in lints the CI toolchain 1.94 does not report).

L11. Evidence (flaky, not this change): two full-suite runs aborted on `tests/update_native.rs::current_version_and_checking_are_flushed_before_the_network_response` and its 10 s window. The fixture copies the 211 MB debug binary once per test, five in parallel, so the window is I/O bound. The same full suite on `HEAD` without this change passed 3437/0, that test passed standalone on `HEAD` and on this branch, and the whole suite then passed 3446/0. Nothing in this diff touches the update path.

L12. "Quiero que siempre el modal tenga margenes por arriba y por abajo del texto que contiene, para que no quede todo amontonado"

L13. Decision (answers L12): the field modal layout is border, document header, blank margin, value, optional refusal, blank margin, footer, border — six chrome rows around the value. `render_json_editor` now takes `Option<&str>` for its footer, so the field modal draws its own footer under the bottom margin while the document EJSON view and the SQL JSON editor keep theirs. `DocumentFieldInput::modal_body_height` became `modal_value_rows`, which counts the value and its refusal only; the chrome lives in `FIELD_MODAL_CHROME_ROWS`.

L14. "Ahora en campo date no lo formatea correctamente."

L15. Decision (answers L14, user chose between three representations): a BSON date shows as ISO-8601 UTC with an explicit `Z` in the field list and the modal, because it is the instant BSON stores and leaves no timezone doubt. `DocumentFieldKind::DateTime` parses RFC 3339, `YYYY-MM-DD HH:MM[:SS[.fff]]` and `YYYY-MM-DD` (all without an offset read as UTC) and still accepts canonical EJSON, so a value pasted from the EJSON view round-trips. `compact_bson_value` formats the date, so the list row and the modal share one text. The document text keeps canonical EJSON, so the write path and the EJSON view are unchanged.

L16. Upstream integration (2026-10-30): reuse PR #26 and the date/docs slices, alongside the independent #28 collection-activation fix. Bound the field popup to the document popup and render the value only in its allocated layout rectangle. Long values scroll; dimensions saturate before conversion to terminal coordinates and widths use projected display cells. Small terminals omit margins/context before sacrificing the editable value. Scalar navigation applies locally before switching, retains invalid input, skips `_id`, and stops at either end. Date display falls back to canonical EJSON outside the supported ISO range; text input rejects submillisecond precision and leap seconds. The adapter/runtime compare-and-replace implementation remains untouched.
