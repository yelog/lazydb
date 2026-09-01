# SQL Editor Contextual Completion Design

**Status:** Approved

**Date:** 2026-08-31

## Summary

Make SQL Editor completion filter candidates by the syntactic role at the
cursor. Expression positions such as a `SELECT` projection, `WHERE`, `ON`,
`HAVING`, `GROUP BY`, `ORDER BY`, and `RETURNING` suggest columns from relations
visible in the current SQL scope, expression-capable routines, and keywords
valid for that position. They do not suggest databases, schemas, tables, or
views.

Relation positions continue to suggest databases, schemas, tables, and views.
Qualified completion distinguishes relation aliases from catalog qualifiers so
`alias.` suggests only columns while `schema.` continues to suggest relations.

## Problem

The current completion engine classifies most cursor locations as
`Context::General`. Only relation, qualified, and routine contexts apply a kind
filter. General completion therefore mixes every catalog object whose name
matches the prefix.

There is one additional column-specific filter: unqualified columns are limited
to relations found by `relation_bindings`. It does not apply to relation
candidates. Consequently:

- `WHERE ` with an empty prefix scans the complete catalog and often fills the
  ten-row popup with alphabetically sorted tables;
- `SELECT u` can mix a statement-level keyword, columns from the selected
  relation, and unrelated tables beginning with `u`;
- global catalog entries can displace useful columns before the result is
  truncated.

This behavior is consistent with the implementation but not with the semantic
role of these cursor positions.

## Goals

- Exclude databases, schemas, tables, and views from expression completion.
- Prefer columns belonging to relations visible at the cursor.
- Retain functions and keywords that are legal and useful in the current
  expression context.
- Preserve relation completion after `FROM`, `JOIN`, `UPDATE`, and `INTO`.
- Preserve database, schema, relation, and alias-qualified completion.
- Handle incomplete SQL without requiring a successful full-AST parse.
- Improve relation binding for joins, aliases, nested scopes, comments, quoted
  identifiers, and strings.
- Filter before sorting and limiting candidates.

## Non-Goals

- Full semantic validation or type inference.
- Resolving output aliases or function return types.
- Guaranteeing perfect completion for every vendor-specific SQL extension.
- Replacing `sqlparser` elsewhere in the SQL subsystem.
- Loading the entire database catalog solely for completion.

## Candidate Policy

Replace the broad general context with position-oriented contexts:

```rust
enum CompletionPosition {
    Statement,
    Relation,
    Expression(ExpressionPosition),
    Qualified(QualifierKind),
    Routine,
}
```

`ExpressionPosition` differentiates projection, predicate, grouping, ordering,
and returning positions when the distinction changes the useful keyword set.
`QualifierKind` distinguishes a relation alias or relation name from a database
or schema path.

Candidate kinds are allowed as follows:

| Position | Columns | Functions | Keywords | Tables/views | Database/schema |
| --- | --- | --- | --- | --- | --- |
| Statement start | No | No | Statement | No | No |
| Projection | Visible | Yes | Projection/expression | No | No |
| Predicate | Visible | Yes | Predicate/expression | No | No |
| Grouping | Visible | Limited | Grouping/expression | No | No |
| Ordering | Visible | Limited | Ordering/expression | No | No |
| Returning | Visible | Yes | Returning/expression | No | No |
| Relation | No | No | Relation | Yes | Yes |
| Relation alias/name qualifier | Bound relation | No | No | No | No |
| Database/schema qualifier | No | No | No | Yes | Child catalog nodes |
| Routine | No | Functions/procedures | Routine | No | No |

The policy is a strict allowlist. In particular, an expression position does
not fall back to global relation candidates when relation metadata is missing.

## Context Analysis

Completion must work while the statement is incomplete, so a successful
`sqlparser` parse cannot be a prerequisite. Continue to use `scan_statements`
to isolate the current statement, then analyze it with a lightweight,
dialect-aware token scanner.

The scanner must:

- ignore keywords inside single-quoted strings, quoted identifiers, line
  comments, and block comments;
- preserve identifier text and qualification components;
- track parenthesis depth;
- identify clause transitions relevant to completion;
- collect relations introduced by `FROM`, comma-separated `FROM` items, `JOIN`,
  `UPDATE`, and `INTO`;
- collect explicit aliases with and without `AS` without treating following SQL
  keywords as aliases;
- identify the innermost query scope containing the cursor;
- tolerate unfinished identifiers, clauses, and parenthesized expressions.

The analysis result should separate cursor policy from visible bindings:

```rust
struct CompletionAnalysis {
    position: CompletionPosition,
    bindings: Vec<RelationBinding>,
}

struct RelationBinding {
    name: Vec<String>,
    alias: Option<String>,
    scope_depth: usize,
}
```

The exact internal representation may remain private and smaller if tests prove
the same behavior.

## Scope Rules

The innermost query block is the primary completion scope. Its local relation
bindings are visible. Correlated subqueries may additionally see bindings from
enclosing query scopes, while sibling or completed nested scopes must not leak
bindings into the cursor scope.

For the first implementation, lexical nesting may model this visibility; full
SQL name-resolution semantics are not required. The important invariant is that
an unrelated relation mentioned elsewhere in the statement does not make its
columns visible at the cursor.

CTE names and derived-table output columns require inferred schemas that are not
currently available from the catalog index. They may be recognized as relation
bindings without fabricated columns. Their presence must not cause fallback to
global table suggestions.

## Catalog Resolution

Resolve each binding to catalog relation IDs using this order:

1. exact fully qualified path;
2. exact path relative to the active database and schema;
3. unique object-name match within the active catalog scope;
4. all plausible object-name matches if ambiguity remains.

This avoids silently choosing the wrong table when multiple schemas contain the
same relation name. Ambiguous bindings may contribute columns from every
plausible relation; duplicate candidates are deduplicated by relation and column
identity.

Alias-qualified completion resolves only the binding carrying that alias.
Relation-name qualification resolves matching visible bindings before treating
the qualifier as a catalog path. Database and schema qualifiers continue to
walk catalog children.

## Candidate Collection

Candidate collection follows this order:

1. derive the current identifier prefix and replacement range;
2. analyze the current statement and cursor position;
3. select candidate sources using the position allowlist;
4. restrict columns to resolved visible bindings;
5. apply identifier matching;
6. deduplicate candidates;
7. score and sort candidates;
8. truncate to ten rows.

Filtering before sorting and truncation prevents unrelated global objects from
occupying popup capacity. Empty expression prefixes enumerate visible columns,
not every catalog entry.

## Keywords

Replace the single global keyword list with context-specific sets. Initial sets
should remain intentionally small and high-value:

- statement: `SELECT`, `WITH`, `INSERT`, `UPDATE`, `DELETE`;
- projection/expression: `DISTINCT`, `CASE`, `NULL`, `TRUE`, `FALSE`;
- predicate: `AND`, `OR`, `NOT`, `EXISTS`, `IN`, `IS`, `NULL`, `LIKE`, `BETWEEN`,
  `CASE`, `TRUE`, `FALSE`;
- grouping: `GROUP BY`, `HAVING` where appropriate;
- ordering: `ASC`, `DESC`, and dialect-supported null ordering;
- relation: relation modifiers supported by the dialect;
- routine: routine-specific keywords where useful.

Dialect-specific sets can add or remove entries. Statement-level `UPDATE`, for
example, must not appear while completing a `SELECT` projection merely because
its name matches the prefix.

Multi-word keywords may initially insert their complete text as one candidate.
They do not require snippet placeholders.

## Functions And Procedures

Expression-capable function catalog entries remain available in expression
positions. Procedures remain restricted to routine invocation contexts. SQLite
keeps its existing exclusion when routine metadata is unsupported.

The initial change does not infer function signatures or append parentheses.

## Ranking

Position filtering is authoritative and precedes ranking. Within allowed
candidates, score using:

1. contextual usefulness;
2. identifier match quality: exact, ordinary prefix, compact prefix;
3. active database/schema preference where relevant;
4. deterministic label ordering.

Visible columns should lead an empty-prefix expression popup. For a non-empty
prefix, an exact or stronger function/keyword match may rank ahead of a weaker
column match when useful. Ranking must not reintroduce disallowed candidate
kinds.

## Metadata Loading And Failure

Existing relation-ID discovery is used to request relation children when a
visible relation has no loaded columns. Missing metadata is non-blocking:

- editing and execution continue normally;
- the popup may contain only legal keywords and functions until columns arrive;
- completion refreshes after catalog children are loaded;
- load failure does not fall back to global table suggestions.

If analysis cannot determine a position safely, prefer a conservative candidate
set over the current all-catalog general set. A relation keyword immediately
before the cursor can still select relation completion; otherwise uncertain
positions should offer statement or expression keywords without unrelated
catalog objects.

## Compatibility

The public `complete` API and `CompletionCandidate` rendering contract should
remain unchanged unless implementation proves a small internal API extraction
necessary. Relation insertion, identifier quoting, compact identifier matching,
terminal sanitization, popup lifecycle, and the ten-candidate limit remain
unchanged.

No backward-compatibility mode is required for the old mixed-candidate behavior.

## Testing

Extend `tests/sql_completion.rs` with behavior-level coverage:

- `SELECT u` from a known relation contains matching visible columns and no
  database, schema, table, or view candidates;
- `WHERE ` contains visible columns and predicate keywords but no relation
  candidates;
- statement-level `UPDATE` is absent from a `SELECT` projection;
- `JOIN ... ON` sees columns from both join sides;
- comma-separated `FROM` relations are visible;
- unqualified completion combines visible columns and deduplicates correctly;
- `alias.` contains only that alias's relation columns;
- `schema.` still contains relations;
- active database/schema resolves duplicate relation names preferentially;
- ambiguous relation names do not select an arbitrary catalog relation;
- nested and correlated subqueries obey lexical scope visibility;
- strings, quoted identifiers, line comments, and block comments do not create
  false bindings or clause transitions;
- missing relation children do not restore global relation candidates;
- filtering occurs before the ten-candidate limit;
- PostgreSQL, MySQL, SQLite, identifier quoting, and compact matching do not
  regress.

Focused unit tests may be added for the private scanner and context analyzer if
behavior-level tests cannot isolate malformed or nested-input cases clearly.

## Delivery

Implement in two cohesive stages:

1. Introduce position allowlists and context-specific keywords, immediately
   removing relation objects from expression completion.
2. Replace the whitespace-based relation binding parser with the tolerant token
   scanner and scope-aware binding resolution.

Both stages follow this design. Stage one resolves the reported behavior; stage
two makes the same policy reliable for complex SQL rather than adding a separate
compatibility path.

## Acceptance Criteria

- The two reported `SELECT` and `WHERE` cases no longer show table candidates.
- Expression completion offers only visible columns, expression-capable
  functions, and position-valid keywords.
- Relation completion and catalog qualification continue to work.
- Alias-qualified completion contains only columns from the bound relation.
- JOIN and nested-query scopes do not leak unrelated columns.
- Missing metadata never causes a fallback to global relation suggestions.
- Existing quoting, matching, popup, and dialect behavior remains covered and
  passing.
