# SQL Editor Semantic Assistance Design

**Status:** Approved on 2026-08-27

## Goal

Provide hierarchy-aware SQL completion with icon and column-type presentation, load relation columns on demand from the current statement, and underline the exact statement that Run Current SQL will execute.

## Completion Context

Completion input is modeled as a replace range, prefix, and zero or more qualifier segments. The index retains databases in addition to schemas, relations, columns, and routines, and resolves paths by database/schema/relation identity rather than the first matching object name.

Relation positions accept databases, schemas, tables, and views. `database.` returns only schemas in that database; `schema.` returns only tables/views in the current target database; `database.schema.` returns only tables/views in that exact namespace. Default target database/schema affects ranking but never bypasses the visible-object scope.

## Relation And Alias Scope

A tolerant token scan over the current statement extracts FROM, JOIN, UPDATE, and INTO relation bindings and aliases even when the SQL is incomplete. Unqualified expression completion draws columns from those bindings. Alias-qualified completion resolves only the aliased relation. Complete SQL may later use an AST enhancement, but the first implementation does not depend on a successful full parse.

Relation children are loaded on demand for relations referenced by the current statement. Completion requests use a dedicated catalog intent, deduplicate with existing requests, do not expand Explorer nodes, and re-run only if document revision, connection identity, and catalog generation remain current.

## Popup Presentation

`CompletionKind` includes Database. `IconSet` owns icons for every completion kind in Nerd Font, Unicode, and ASCII modes. Popup rows render icon, primary label, and muted detail as separate spans. Column detail is `ColumnMetadata.native_type`; when multiple relations contribute an ambiguous name, a muted source relation is appended.

## Current Statement Decoration

The underline range reuses the same `resolve_scope` logic as Run Current SQL. Syntax color and current-statement decoration are orthogonal: render spans retain syntax kind and gain a semantic decoration flag. The UI applies `Modifier::UNDERLINED` to the meaningful current statement range.

Visual selection remains the stronger execution scope and uses its existing background without a competing underline. No range is shown in relation tabs, prompt overlays, or whitespace/comment gaps with no executable statement. The behavior follows `Action::RunActiveSql`, independent of which key invokes it; the current repository binding remains `Space r`.

## Testing

Completion tests cover database/schema/relation paths, duplicate schema names across databases, aliases, unqualified columns, type details, and missing relation children. Reducer/runtime tests cover completion-triggered relation-child loading and stale request rejection. UI tests cover all icon modes and popup detail styling. Editor/scope/UI tests prove the underlined range exactly matches the SQL dispatched by Run Current SQL across Unicode, comments, quoted semicolons, dollar quotes, multiple statements, visual selection, and horizontal scrolling.
