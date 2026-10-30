# MongoDB collections as relation tabs

Feature: a MongoDB collection opens in the same relation tab as a SQL table, with a DATA grid and a structure view, plus a document edit modal.
Approach B (decided): BSON maps onto the existing `CellValue` variants; no shared contract type changes (`CellValue`, `RelationPreview.sql`).

## Specs

S1. "cuando entro a una tabla que en mongodb es una coleccion, me muestra una pestaña DATA con todas las filas y otra pestaña DDL que me muestra la estructura. Quiero que mongo copie esta forma."

S2. "En MongoDB no existe DDL" — the structure tab shows, per the user's choice ("Las dos cosas"): real metadata (collection options, validator when present, indexes) and a schema inferred from the loaded page (field, observed types, presence percentage).

S3. "El edit me abre el documento en formato json. Lo que queria es que se abra cada campo y editar por propiedad. No se si es posible... pero al menos me gustaria que se formatee correctamente." The modal opens on the per-field view so a document is edited by property, the Canonical EJSON view stays available from inside the same modal, and that EJSON is rendered formatted across lines instead of one clipped line.

S4. "Tambien en la lista de filas me marca en el campo donde estoy parado." Per the user's choice ("Celda actual resaltada, flechas entre campos"): the grid tracks row and column, the active cell is highlighted, and arrow keys move between fields.

S5. Constraint from approach B: `src/db/value.rs` `CellValue` and `src/db/mod.rs` `RelationPreview` are not modified. BSON scalars map to existing variants; `ObjectId` becomes hex text; documents and arrays become shape markers (`{n fields}`, `[n]`) with the full canonical EJSON available in the modal.

## Tasks

- T1 (S1) — Open a MongoDB collection as a `RelationTab` whose DATA grid lists documents, with BSON mapped to `CellValue` and paging through `FindSpec`. Route: worker. Commit: `feat(mongodb): open collections in a relation tab`.
- T2 (S4) — Cell cursor over the document grid: row and column navigation, active cell highlight, active field visible in the status line. Route: inline. Commit: `feat(mongodb): track the active cell in the document grid`.
- T3 (S2) — STRUCTURE view content: collection options, validator, indexes, and inferred schema of the loaded page. Route: worker. Commit: `feat(mongodb): show collection structure`.
- T4 (S3) — Edit modal with the whole document in Canonical EJSON and a `replaceOne` save bounded to `_id`. Route: worker + verify (high risk: writes data). Commit: `feat(mongodb): edit a document in a modal`.
- T5 (S3) — Field-by-field view inside the edit modal. Route: inline. Commit: `feat(mongodb): edit document fields individually`.
- T6 (S2) — Infer the structure schema from the page already in memory instead of re-reading the find spec. Route: inline. Commit: `fix(mongodb): infer structure from the loaded page`.

## Log

L1. "espera... Quiero que funcione igual como funciona con las bases de datos postgresql. Me refiero a como se muestran los datos... cuando entro a una tabla que en mongodb es una coleccion, me muestra una pestaña DATA con todas las filas y otra pestaña DDL que me muestra la estructura. Quiero que mongo copie esta forma. El editar en mongo quiero tambien sea como en postgresql que abre un modal. Tambien en la lista de filas me marca en el campo donde estoy parado."

L2. User chose, in the product questionnaire: structure view = "Las dos cosas"; edit modal = "EJSON por defecto y campo por campo disponible"; cursor = "Celda actual resaltada, flechas entre campos".

L3. Architecture survey (Engram observation 6035) found the SQL relation tab assumes: a non-optional `RelationPreview.sql` that is always displayed (`src/ui/relation.rs:856-870`), `CellValue` with no document variant, `open_catalog_relation` requiring a `CatalogId` from `profile.catalog`, and `editable_capability` requiring `CatalogKind::Table` plus a primary key. The grid already implements S4 through `DataGridState { selected_row, selected_column }` and `theme.grid_active_cell()` (`src/ui/data_grid.rs:315-317`), and the DDL view is a read-only text editor that can display generated structure text.

L4. Decision: approach B. `CellValue` and `RelationPreview` stay untouched so no SQL adapter is at contract risk before the data-writing slice lands. Nested-document navigation inside the grid is explicitly out of scope; the modal is the place to read a whole document.

L5. Decision: T1 replaces the explorer path that opens `MongoBrowserTab` for a collection. Until T4 lands, the inline EJSON editing that lives in the current browser is unavailable; the user accepted this sequencing by choosing the modal as the editing surface.

L6. Decision: pagination for T1 reuses `ResultPagination` and the existing `FindSpec` skip/limit budget; `RelationPreview.sql` is filled with a human-readable description of the `find` command actually executed, so the field stays truthful instead of empty.

L7. T3 approved with one surface deviation: `src/model/mod.rs` gained `pub mod mongodb_structure;`, the only way to make the new module reachable from the allowed surfaces. Verified that `src/db/value.rs`, `src/db/mod.rs`, `src/ui/data_grid.rs` are untouched and that the `src/ui/relation.rs` diff belongs to T2 only.

L8. T3 compromise recorded: `src/db/mongodb/read.rs` was outside its surfaces, so the inferred schema comes from re-running the same find spec instead of reading the page already held by the tab. Consequences: one extra read when the DDL view opens, and a report that can reflect documents written after the grid loaded. T6 removes the compromise.

L9. T4 landed after the delegated worker stalled for 30 minutes and left one failing test. Audit by the parent found the write path correct (single `replaceOne` bounded to `{_id: original}` in `src/db/mongodb/mutation.rs`, `_id` immutability guard, `Unknown` outcome on ambiguous network errors with no retry) and 22 of 23 tests passing. The single failure was a fixture defect, not product code: the integration test imported a server URL with no database, so the profile's target failed `ExecutionTarget::is_valid` and `RequestConnect` returned no command. Fixed by importing the URL with `/lazydb_demo`.

L10. T4 was verified against a real server, never the user's Atlas cluster: local MongoDB from `docker-compose.mongodb.yml` on 127.0.0.1:27018 via colima, database `lazydb_demo`, collection `bikes`. The integration test asserts the edited document changed, the other two stayed byte-identical, the reloaded grid shows the saved value, and it restores the demo documents before asserting so a failure cannot leave the shared collection modified.

L11. T5 landed: `Tab` toggles the document modal between the Canonical EJSON text and a FIELDS list (name, BSON type, compact value). `j`/`k` move, `Enter` edits the selected field, `Ctrl+S` (or `Enter` in the single-line input) applies it into the EJSON text, `Esc` leaves the field edit and a second `Esc` closes the modal. `_id` is listed but never editable. The EJSON text stays the single source of truth and the T4 write path is unchanged.

L12. Accepted T5 compromises: applying a field re-renders the document in canonical form, so hand formatting in the EJSON text is normalized (BSON types survive the round trip); string fields are edited raw, distinguished from null by the type column; `src/help.rs` was intentionally not extended because the modal shares `ShortcutContext::RelationDataEdit` with the SQL cell editor, so discovery lives in the modal footer.

L13. Requirement change: the user reported that the modal opened on one clipped EJSON line, and asked for per-property editing with correct formatting. The earlier choice ("EJSON por defecto y campo por campo disponible") is superseded: the modal now opens on the field view, and `canonical_document_ejson` renders the document indented across lines, which also applies after applying a field. `Tab` still switches to the EJSON view and back.

L14. Requirement change: the user asked to hide the `Monitor · <database>` and `Users & Roles · <database>` rows so the tree shows only databases and their collections. Both rows were removed from the projection and their two features are now reached from a selected database row with `m` (monitor) and `u` (users and roles), through the existing `Action::OpenMongoNode`, so no capability was lost. The projection test now pins that no action row is listed, and a keymap test pins both keys.

L15. Delivery shape: the whole feature landed in one commit (6783 authored lines) instead of the four reviewable slices T1-T5 proposed, and this is the recorded reason. The slices are interleaved inside `src/app.rs`, `src/input/keymap.rs` and `src/ui/relation.rs`, so a per-slice split would have required inventing intermediate states that never compiled together; an artificial but green-looking history was judged worse than one honest commit whose message enumerates the slices and their tests. Future work on this area should split by file ownership before writing, not after.

L16. Delivery chain, built on top of yelog/lazydb main (25e23cf) as five reviewable commits, each one compiled and tested on its own branch before the next started:
- `34575f7` fix(mongodb): expand databases and collections in the explorer — 513 lines, 7 files.
- `5989a05` feat(mongodb): show collection documents as a field grid — 527 lines, 7 files.
- `6359816` feat(mongodb): open collections as relation tabs — 1036 lines, 8 files.
- `75fe966` feat(mongodb): show the collection structure — 1160 lines, 8 files.
- feat(mongodb): edit a document in a modal — the last commit of the chain; its tree equals the earlier single-commit state plus the clippy fix required by upstream's `-D warnings` gate.
Verified at the tip: build green, mongodb_relation 33, keymap 166, ui_render 243, explorer_state 48, mongodb_explorer 7, relation_tabs 50, fmt clean, and no clippy warning in any touched file. The 13 DuckDB lib tests fail on this machine for a missing `libduckdb.dylib`, reproduced on a clean checkout of upstream.
