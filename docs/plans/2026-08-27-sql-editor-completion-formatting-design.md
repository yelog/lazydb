# SQL Editor Completion And Formatting Design

**Status:** Approved on 2026-08-27

## Goal

Make completion an Insert-mode-only editor feature, expose and verify SQL formatting for the selected or current statement, show fully qualified relation names in the popup, and insert the shortest useful dialect-aware relation reference for the active SQL execution target.

## Problems And Root Causes

Every Vim buffer mutation currently emits `EditorEffect::Changed`. `App::apply_editor_effects` schedules completion for that effect without checking the editor mode, so Normal-mode commands such as `dd`, `x`, and `p` can create a popup. The delayed completion key validates the document revision, connection, and catalog generation, but not the editor mode. Completion calculation, acceptance, and rendering also lack a mode guard.

SQL formatting already exists through `Space f` and `:format`. `App::format_current` uses the shared scope resolver, so a contiguous Visual selection wins and otherwise the statement at the cursor is selected. The remaining work is to make the entry discoverable, preserve the existing safety checks, cover the behavior with integration tests, and ensure formatting outside Insert mode cannot trigger completion.

Catalog entries already carry `QualifiedName { database, schema, object }`, but completion constructs both `label` and `insert_text` from `object` alone. The completion API receives only a default schema for ranking and therefore cannot compare a candidate with the active editor database and schema.

## Insert-Only Completion Invariant

Only `EditorMode::Insert` may create, retain, render, navigate, or accept a completion popup.

The invariant is enforced at each state boundary:

- `apply_editor_effects` clears completion and does not schedule it when a change finishes outside Insert mode.
- `completion_key` returns `None` outside Insert mode, invalidating delayed `CompletionDue` actions after Escape or another mode transition.
- `complete_now` refuses to calculate or store candidates outside Insert mode.
- `accept_completion` refuses to edit the document outside Insert mode.
- Popup rendering checks the mode defensively and never projects invalid state.

`EditorEffect::Changed` remains mode-independent because revision tracking, persistence, and other editor behavior still need to observe Normal- and Visual-mode mutations. Completion policy belongs in the App layer rather than the editor mutation event.

## SQL Formatting

Keep the existing `Space f` and `:format` entry points. Add the shortcut to the searchable Help palette with wording that explains the selected/current behavior.

Formatting scope follows the existing execution scope model:

- Visual Char formats the contiguous selected range.
- Visual Line formats the selected complete-line range.
- Visual Block remains unsupported because a rectangular selection is not one contiguous SQL fragment; the editor preserves the buffer and shows the existing explanatory message.
- Without a Visual selection, the statement containing the cursor is formatted.
- Whitespace and comment gaps with no SQL scope preserve the buffer and show `No SQL scope at cursor`.

Keep the formatter's token-equivalence validation and dollar-quoted procedural-body protection. Any formatting or validation error preserves the original SQL and is shown to the user. A successful replacement participates in normal editor undo/revision behavior, while the Insert-only completion invariant prevents formatting in Normal or Visual mode from opening a popup.

## Completion Context

Replace the `default_schema` argument with a lightweight completion context containing the active execution target database and schema. The SQL module should not depend on the full `ExecutionTarget` model because it only needs namespace values for ranking and insertion.

Conceptually:

```rust
pub struct CompletionContext<'a> {
    pub database: Option<&'a str>,
    pub schema: Option<&'a str>,
}
```

The active schema continues to raise the ranking of local candidates. Database and schema comparisons are case-insensitive, matching the existing completion lookup behavior.

## Relation Candidate Presentation

Table, view, and materialized-view labels always show the complete catalog identity, even for the active namespace:

```text
app.public.users
app.audit.users
analytics.reporting.users
```

This keeps same-named relations distinguishable. Database, schema, and object components are sanitized independently with `display_text` before joining them. Keywords, columns, and routines retain their current presentation; column detail remains the native type.

Sorting remains score-first. The complete label provides a deterministic tie-breaker for duplicate object names.

## Relation Insertion

Insertion is separate from presentation. Each required identifier component is quoted independently with the active SQL dialect and then joined with dots.

For a generic three-level namespace and active target `app.public`:

| Candidate | Insert text |
| --- | --- |
| `app.public.users` | `users` |
| `app.audit.events` | `audit.events` |
| `analytics.bi.metrics` | `analytics.bi.metrics` |

If the user has already typed a qualifier, completion replaces only the current identifier prefix and inserts the remaining object component. Accepting `app.public.users` after `app.public.us` therefore produces `app.public.users`, not a duplicated path.

Dialect rules are centralized in one relation insertion helper:

- PostgreSQL uses `table` in the active schema, `schema.table` in another schema of the active database, and keeps the requested `database.schema.table` insertion for a catalog entry in another database. Cross-database acceptance does not switch the execution target; PostgreSQL may reject the SQL when it is run.
- MySQL models database and schema as the same namespace. It inserts `table` in the active database and `database.table` in another database, never `database.database.table`.
- SQLite uses the attached database/schema alias and inserts `table` in the active schema or `schema.table` in another attached schema. It does not insert the database file path as a third qualifier.
- Generic uses the three-level database/schema/object rules.

Explicitly typed qualifiers remain authoritative. Qualified lookup continues to resolve only children under the typed path, while generated insertion avoids repeating that path.

## Error Handling

Completion with incomplete namespace metadata falls back conservatively to the available components and never emits empty path segments. Identifier quoting remains independent from terminal-safe display sanitization so hostile display text cannot alter the raw identifier selected for insertion.

Formatting errors and unsupported Visual Block selections preserve the original buffer. Completion mode guards silently clear stale popup state because a mode transition is normal editor behavior, not an error.

## Testing

Completion lifecycle tests cover Insert-mode input, delayed completion followed by Escape, Normal-mode `dd`, `x`, and `p`, stale popup cleanup, and rejection of acceptance outside Insert mode.

Formatting tests cover `Space f`, `:format`, Help discovery, current-statement-only replacement, Visual Char and Visual Line replacement, Visual Block rejection, whitespace/comment gaps, formatter errors, undo/revision behavior, and the absence of a popup after formatting outside Insert mode.

Completion engine tests cover complete relation labels, local bare insertion, same-database cross-schema insertion, cross-database insertion, already typed qualifiers, independent quoting of every identifier component, duplicate object names, hostile display text, PostgreSQL cross-database acceptance without target changes, MySQL namespace deduplication, and SQLite attached-schema qualification.

UI tests verify that non-Insert modes never render completion and that complete relation labels determine popup width without breaking terminal sanitization or selection styling.
