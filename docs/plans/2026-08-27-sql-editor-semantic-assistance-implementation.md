# SQL Editor Semantic Assistance Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add qualified catalog completion, alias-aware typed column completion with icons, completion-driven relation metadata loading, and current-statement underline decoration.

**Architecture:** Parse completion input and current-statement relation bindings into explicit semantic context, resolve catalog paths through indexed identities, and load only missing relation children. Keep popup icon rendering in `IconSet` and current-statement decoration separate from syntax highlight kinds while sharing the execution scope resolver.

**Tech Stack:** Rust 2024, SQLx 0.9, sqlparser 0.62, Ratatui 0.30, Modalkit, Tokio, Cargo test, Clippy

---

### Task 1: Index Databases And Qualified Catalog Paths

**Files:**
- Modify: `src/sql/completion.rs:17-107,139-262,290-359`
- Modify: `tests/sql_completion.rs`

**Step 1:** Add failing tests for Database candidates after `FROM`, schemas after `database.`, relations after `schema.` and `database.schema.`, and duplicate `public` schemas in different databases.

**Step 2:** Run `cargo test --test sql_completion --all-features`; expect failures because Database is filtered and qualifiers hold one ambiguous name.

**Step 3:** Add `CompletionKind::Database`, path indexes, and `QualifiedInput { replace, prefix, qualifiers }`. Resolve parent identities using complete paths and current target database/schema.

**Step 4:** Permit Database/Schema/Table/View in relation context and rank current target relations first.

**Step 5:** Run the focused suite; expect all hierarchy tests to pass without changing raw identifier insertion/quoting.

### Task 2: Parse Current Statement Relation Bindings

**Files:**
- Modify: `src/sql/completion.rs`
- Modify: `src/sql/mod.rs`
- Test: `tests/sql_completion.rs`
- Reference: `src/sql/scope.rs`

**Step 1:** Add failing tests for `SELECT u| FROM sys_user`, `SELECT u.| FROM sys_user u`, JOIN aliases, schema-qualified relations, and same-named columns from two relations.

**Step 2:** Implement tolerant token-based extraction of FROM/JOIN/UPDATE/INTO relation paths and optional aliases inside the current statement range.

**Step 3:** Resolve bindings through the qualified completion index. Use bound relation children for expression candidates and alias-qualified children for qualifier candidates.

**Step 4:** Keep keyword fallback when no relation resolves or SQL is incomplete beyond recognition.

**Step 5:** Run `cargo test --test sql_completion --all-features` and verify column candidates carry `native_type` detail.

### Task 3: Load Missing Relation Children For Completion

**Files:**
- Modify: `src/app.rs:90-105,998-1028,2940-3080,3530-3787`
- Modify: `src/action.rs`
- Modify: `src/model/explorer.rs`
- Test: `tests/catalog_reducer.rs`
- Test: `tests/sql_completion.rs`

**Step 1:** Add reducer tests where SQL references a loaded table whose RelationChildren are not loaded. Assert completion emits one dedicated request and does not expand the Explorer relation.

**Step 2:** Add `CatalogRequestIntent::Completion` and a completion-pending descriptor keyed by console/document revision, connection, catalog generation, and relation IDs.

**Step 3:** Before producing final candidates, request missing RelationChildren for resolved bindings. Reuse existing pending requests and never fan out for unrelated tables.

**Step 4:** On response, rebuild `CompletionIndex` and rerun completion only when the pending descriptor is still current. Ignore stale responses for popup reopening while still accepting valid catalog data.

**Step 5:** Run catalog reducer, connection switch, and SQL completion tests.

### Task 4: Render Completion Icons And Typed Details

**Files:**
- Modify: `src/ui/icons.rs`
- Modify: `src/ui/mod.rs:268-322`
- Test: `src/ui/icons.rs`
- Test: `tests/ui_render.rs`

**Step 1:** Add icon mapping tests for every `CompletionKind` in Nerd Font, Unicode, and ASCII modes.

**Step 2:** Add popup render tests asserting icon, label, and column type appear, and details use muted style in selected and unselected rows.

**Step 3:** Implement `IconSet::completion(kind)` using named Nerd Font constants and safe fallbacks.

**Step 4:** Render popup rows with separate spans and include icon width in popup measurement. Preserve terminal sanitization and selection contrast.

**Step 5:** Run icon and UI render tests.

### Task 5: Project Current Statement As A Semantic Decoration

**Files:**
- Modify: `src/sql/scope.rs`
- Modify: `src/sql/mod.rs`
- Modify: `src/model/editor.rs:38-58`
- Modify: `src/editor/mod.rs:394-470,1450-1535`
- Modify: `src/app.rs:245-254`
- Modify: `src/ui/mod.rs:620-776`
- Test: `tests/sql_scope.rs`
- Test: `src/editor/tests.rs`
- Test: `tests/ui_render.rs`

**Step 1:** Add tests proving a public current-statement range exactly matches `resolve_scope(...).source` for multiple SQL statements, Unicode, comments, and quoted/dollar-quoted semicolons.

**Step 2:** Add a semantic decoration field to `EditorRenderSpan` without adding a syntax highlight kind. Split spans at decoration boundaries during render snapshot construction.

**Step 3:** Have App pass the current execution range into snapshot rendering only when there is no Visual selection or prompt.

**Step 4:** Apply `Modifier::UNDERLINED` while preserving syntax foreground, selection background, viewport clipping, and horizontal scrolling.

**Step 5:** Add UI tests that compare the underlined source range with the SQL emitted by `Action::RunActiveSql`.

### Task 6: Verify And Document

**Files:**
- Modify: `README.md`
- Modify: `docs/keybindings.md`
- Reference: `docs/plans/2026-08-27-sql-editor-semantic-assistance-design.md`

Run:

```bash
cargo fmt --check
cargo test --test sql_completion --all-features
cargo test --test catalog_reducer --all-features
cargo test --test sql_scope --all-features
cargo test --test ui_render --all-features
cargo test --test keymap --all-features
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
git diff --check
```

Expected: all checks pass. Manually verify hierarchy completion, alias columns with types, all icon modes, and that the underlined statement exactly matches `Space r` execution. Keep `Space d` documented as target selection unless a separate keymap change is explicitly approved.
