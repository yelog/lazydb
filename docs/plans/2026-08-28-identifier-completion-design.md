# Identifier Completion Design

## Goal

Improve identifier completion in two related areas:

- SQL Editor identifiers use case-insensitive, separator-insensitive prefix matching, so `sysuser` can match `sys_user`.
- Relation preview `WHERE` and `ORDER BY` inputs automatically suggest columns belonging to the current relation.

The fuzzy boundary is intentionally narrow. Matching ignores `_`, `-`, and ASCII whitespace, but does not implement arbitrary subsequence matching, edit-distance typo correction, or generic punctuation removal.

## Current State

SQL completion in `src/sql/completion.rs` folds names to lowercase and applies `starts_with`. Its `CompletionIndex` is keyed only by the folded original name. This supports `sysu -> sys_user`, but not `sysuser -> sys_user`.

Relation and SQL result query bars share `DataQueryState` and two `TextInput` values. They have no completion state or popup interaction. `Enter` submits the query, while `Tab` switches between `WHERE` and `ORDER BY`.

Relation preview result columns are not a sufficient primary source for suggestions. `ResultSet.columns` may be absent for a zero-row result because column metadata is currently initialized from streamed rows. Catalog relation children remain available independently of result rows and include native column types.

## Architecture

Share only the behavior that is genuinely common:

- identifier normalization and match classification;
- completion-list presentation primitives where practical.

Keep completion contexts separate:

- SQL Editor retains SQL parsing, relation bindings, qualifiers, schema preference, quoting, and its existing completion lifecycle;
- the query bar uses a lightweight current-relation column completion model and does not construct synthetic SQL or invoke the full SQL completion engine.

This avoids duplicating match semantics without coupling a single-line relation filter to the SQL Editor's document model.

## Identifier Matching

Introduce reusable identifier matching functions under `src/sql/`:

```rust
fn fold_identifier(value: &str) -> String;
fn compact_identifier(value: &str) -> String;
fn identifier_match(candidate: &str, query: &str) -> Option<IdentifierMatch>;
```

`fold_identifier` performs Unicode lowercase folding consistent with current completion behavior. `compact_identifier` additionally removes `_`, `-`, and ASCII whitespace.

Match quality is ordered as follows:

1. exact case-insensitive match;
2. ordinary case-insensitive prefix;
3. compact prefix after separator removal.

Examples:

| Query | Candidate | Result |
| --- | --- | --- |
| `sys_user` | `sys_user` | exact |
| `sys_` | `sys_user` | ordinary prefix |
| `sysu` | `sys_user` | ordinary prefix |
| `sysuser` | `sys_user` | compact prefix |
| `syusr` | `sys_user` | no match |
| `user` | `sys_user` | no match |

SQL completion ranking retains context as the primary discriminator. Match quality replaces the current boolean prefix score, followed by schema preference and deterministic label ordering.

## SQL Completion Index

Extend `CompletionIndex` with a compact-name index in addition to the existing folded-name index. Unqualified lookups query both indexes and deduplicate entry positions. Qualified lookups continue to filter the already-small parent child list.

Prefix scans over `BTreeMap` must stop when keys no longer start with the requested prefix. The current lower-bound range can otherwise enumerate every later key before candidate filtering rejects it.

An empty prefix does not scan the compact index separately. Existing contextual empty-prefix behavior remains unchanged.

The original index must be retained because ordinary prefix matches rank above compact matches and multiple original names may map to the same compact name.

## Query Bar Completion Model

Add an optional lightweight completion state to `DataQueryState`:

```rust
pub struct DataQueryCompletion {
    pub candidates: Vec<DataQueryCandidate>,
    pub selected: usize,
    pub replace: TextRange,
}

pub struct DataQueryCandidate {
    pub name: String,
    pub type_name: Option<String>,
}
```

The replacement range is relative to the active `TextInput`. Candidates are limited to ten and ordered by identifier match quality, then case-insensitive name and original name for deterministic output.

Completion refreshes when a query input gains focus or its text/cursor changes. It is cleared when the user switches inputs, submits or cancels the query, changes tabs, or leaves the relevant data view.

A lightweight lexical scan identifies the current identifier around the cursor. Suggestions are suppressed inside string literals. The scanner needs to recognize single-quoted strings and dialect identifier quoting sufficiently for incomplete input; it does not need a complete SQL parse.

Accepted candidates use the existing dialect-aware `quote_identifier` function. Only the identifier under the cursor is replaced, preserving surrounding expressions and sort modifiers.

## Column Source

The primary source is the active profile's completion catalog, restricted to `CatalogKind::Column` entries whose parent is the current relation's `CatalogId`.

Add a controlled `CompletionIndex::relation_columns` accessor so application code does not depend on index internals.

When a relation tab opens and its columns are not present, schedule the existing `CatalogTarget::RelationChildren` load. This reuses current database adapter and catalog reducer behavior, supports empty tables, and also improves later SQL Editor column completion.

Current or previous `RelationPreview` result columns may be used as a temporary fallback while catalog children are unavailable. Catalog entries remain authoritative because result metadata can be missing for zero-row results and catalog metadata includes native types.

Missing or failed column metadata must not block editing, query validation, or submission. The query bar simply remains without suggestions.

## Interaction

Suggestions appear automatically while an identifier is being entered.

When a popup is open:

| Key | Behavior |
| --- | --- |
| `Ctrl+N` | select next candidate |
| `Ctrl+P` | select previous candidate |
| `Tab` | accept selected candidate |
| `Esc` | dismiss only the popup |
| `Enter` | submit the query |

When no popup is open, existing behavior remains:

- `Tab` switches between `WHERE` and `ORDER BY`;
- `Esc` cancels query input;
- `Enter` submits the query.

No new relation-prefixed action aliases are added. New actions use the shared `DataQueryCompletion*` naming because relation-prefixed query actions are retained only as compatibility reducer aliases.

## Rendering

Extract the reusable list rendering and popup placement portions of the SQL Editor completion popup where doing so keeps the API small. SQL Editor and query bar retain separate anchor calculation:

- SQL Editor anchors to its document cursor;
- query bar anchors to the active single-line input cursor and uses the relation/results viewport as its placement boundary.

Query bar rows show the column icon, name, and optional native type. The popup is capped at ten rows, remains inside the viewport, and may render below or above its anchor depending on available space. Candidate labels and details continue to pass through terminal-safe display paths.

## Scope

The first version suggests only current-relation columns. It does not suggest SQL keywords, functions, operators, `ASC`/`DESC`, or `NULLS FIRST`/`NULLS LAST`. This keeps results precise and matches the requested behavior.

SQL-result filtering is not expanded by this change. Column suggestions apply to relation preview query bars where a concrete current relation exists.

## Error Handling

- No columns available: show no popup and preserve normal input behavior.
- Catalog load pending or failed: allow normal editing and submission.
- Candidate becomes stale after metadata refresh: clamp selection or close the popup during recomputation.
- Quoted or otherwise invalid partial input: suppress suggestions rather than changing validation behavior.
- Query validation and database errors remain handled by the existing submission path.

## Testing

### Identifier Matching

Test exact, ordinary prefix, compact prefix, case-insensitive behavior, supported separators, unsupported subsequences, and non-separator punctuation.

### SQL Completion

Extend `tests/sql_completion.rs` to verify:

- `select * from sysuser` suggests `sys_user`;
- ordinary prefix matches rank above compact matches;
- qualified table and alias column completion use the same rule;
- insertion text and dialect quoting remain correct;
- relation, routine, and column context filtering do not regress;
- duplicate results from the two indexes are removed.

### Query Bar Reducer and Keymap

Test both `WHERE` and `ORDER BY`, current-relation filtering, candidate refresh after edits and cursor movement, `Ctrl+N/P`, `Tab` acceptance, two-stage `Esc`, and `Enter` submission while a popup is open.

Test that accepting a candidate replaces only the current identifier and places the cursor after the inserted identifier.

### Column Loading

Test loaded catalog columns, scheduling relation-child metadata when absent, zero-row relation behavior, result-column fallback, and non-blocking behavior when metadata is unavailable.

### UI

Test query-bar popup anchoring, selection style, type display, terminal sanitization, narrow viewport bounds, absence for empty candidates, and no regression to the SQL Editor popup.

## Acceptance Criteria

- Typing `select * from sysuser` can suggest and insert `sys_user`.
- Existing exact and ordinary prefix matches continue to rank ahead of compact matches.
- Relation `WHERE` and `ORDER BY` inputs automatically suggest only current-table columns.
- Empty tables can receive suggestions from Catalog metadata.
- `Ctrl+N/P`, `Tab`, `Esc`, and `Enter` follow the interaction table above.
- Existing query submission, validation, SQL completion context, and completion popup behavior remain intact.
