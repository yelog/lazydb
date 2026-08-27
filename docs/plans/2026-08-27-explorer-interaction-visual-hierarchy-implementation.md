# Explorer Interaction and Visual Hierarchy Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Separate Explorer expansion from activation and render a quieter, semantically styled catalog tree.

**Architecture:** Preserve the normalized Explorer tree, catalog identities, lazy loading, and relation-opening behavior. Split key mappings at the input boundary, expose semantic presentation fields from `VisibleCatalogNode`, and build individually styled Ratatui spans only for visible rows.

**Tech Stack:** Rust 2024, Ratatui, Crossterm, cargo test

---

### Task 1: Separate `o` Toggle From `Enter` Activation

**Files:**
- Modify: `tests/keymap.rs`
- Modify: `src/input/keymap.rs:483-519`

**Step 1: Write the failing keymap assertions**

In the Explorer keymap test, assert:

```rust
assert_eq!(
    keymap.map(key(KeyCode::Char('o')), &app),
    Some(Action::ExplorerToggle)
);
assert_eq!(
    keymap.map(key(KeyCode::Enter), &app),
    Some(Action::ExplorerOpenSelected)
);
```

Retain the existing Editor and Results focus assertions so `o` remains
contextual outside Explorer.

**Step 2: Run the focused test and confirm failure**

Run: `cargo test --test keymap maps_explorer_and_result_actions_by_context`

Expected: FAIL because Explorer `o` currently maps to
`ExplorerOpenSelected`.

**Step 3: Implement the minimal mapping split**

Change the Explorer match arms to:

```rust
KeyCode::Enter => Some(Action::ExplorerOpenSelected),
KeyCode::Char('o') => Some(Action::ExplorerToggle),
```

Do not alter `h`, `l`, `p`, `D`, or mouse behavior.

**Step 4: Run keymap tests**

Run: `cargo test --test keymap`

Expected: PASS.

### Task 2: Prove Toggle Never Opens a Relation

**Files:**
- Modify: `tests/catalog_reducer.rs` or `tests/relation_tabs.rs`

**Step 1: Add reducer coverage**

Build a selected relation node with an owning schema/group and call
`app.update(Action::ExplorerToggle)`. Assert that expansion changes when the
relation is expandable, no relation tab is created, active focus does not move
to Results, and no relation-preview command is emitted.

Also retain or add coverage showing `ExplorerOpenSelected` on a relation and on
one relation descendant opens the owning relation's Data tab.

**Step 2: Run the focused tests**

Run: `cargo test --test catalog_reducer explorer_toggle_never_opens_relation`

Run: `cargo test --test relation_tabs explorer_enter_opens_owning_relation_from_descendant`

Expected: PASS after Task 1 because the reducer already separates
`ExplorerToggle` from `ExplorerOpenSelected`. These are regression tests for the
input contract.

### Task 3: Add User-Facing Count Formatting

**Files:**
- Modify: `src/model/workspace.rs:212-287,391-478`
- Modify: `tests/ui_render.rs`

**Step 1: Write count projection tests**

Create group states with `CatalogCount::Exact(79)`,
`CatalogCount::AtLeast(79)`, and `CatalogCount::Unknown`. Assert projected detail
is respectively `Some("79")`, `Some("79+")`, and `None`.

**Step 2: Run the test and confirm failure**

Run: `cargo test --test ui_render explorer_group_counts_are_user_facing`

Expected: FAIL because current projection uses `Debug` and returns `Exact(79)`.

**Step 3: Implement one formatter**

Add a UI-neutral helper near the projection helpers:

```rust
fn catalog_count_label(count: CatalogCount) -> Option<String> {
    match count {
        CatalogCount::Exact(value) => Some(value.to_string()),
        CatalogCount::AtLeast(value) => Some(format!("{value}+")),
        CatalogCount::Unknown => None,
    }
}
```

Use it for group detail instead of `format!("{:?}", state.count)`.

**Step 4: Run projection and catalog tests**

Run: `cargo test --test ui_render --test catalog_contract --test catalog_reducer`

Expected: PASS.

### Task 4: Remove Redundant Catalog Type Suffixes

**Files:**
- Modify: `src/model/workspace.rs:377-389,462-477`
- Modify: `tests/ui_render.rs:361-397`

**Step 1: Update the projection expectation**

Rename the existing metadata-order test to describe separate label and metadata
semantics. Change the expected database label from `"db  DATABASE"` to `"db"`.
Add table/schema examples proving their labels also contain only object names.

**Step 2: Run the test and confirm failure**

Run: `cargo test --test ui_render explorer_catalog_labels_omit_redundant_native_kinds`

Expected: FAIL because `entry_label` appends `native_kind`.

**Step 3: Simplify the catalog label**

Change `entry_label` to return only the qualified object's display name:

```rust
fn entry_label(entry: &CatalogEntry) -> String {
    entry.qualified_name.object.clone()
}
```

Keep `native_kind` in `CatalogEntry`; only remove it from the Explorer label.

**Step 4: Run UI and Explorer model tests**

Run: `cargo test --test ui_render --test explorer_state --test catalog_reducer`

Expected: PASS.

### Task 5: Expose Semantic Root and Comment Fields

**Files:**
- Modify: `src/model/workspace.rs:140-148,212-287,414-477`
- Modify: `tests/ui_render.rs`

**Step 1: Add projection tests before changing the type**

Cover these semantic outcomes:

- saved profile: no provenance label
- session profile: `SESSION` provenance
- profile row exposes database kind, connection status, and endpoint separately
- catalog row exposes structural metadata separately from comment
- table comment is not concatenated into the primary label

**Step 2: Change `VisibleCatalogNode` minimally**

Replace the overloaded combined `detail` use with the smallest semantic fields
needed by rendering. One acceptable shape is:

```rust
pub struct VisibleCatalogNode {
    pub id: ExplorerNodeId,
    pub depth: usize,
    pub label: String,
    pub metadata: Option<String>,
    pub comment: Option<String>,
    pub kind: Option<CatalogKind>,
    pub profile_kind: Option<DatabaseKind>,
    pub provenance: Option<ProfileProvenance>,
    pub connection_status: Option<ExplorerConnectionStatus>,
    pub endpoint: Option<String>,
    pub expandable: bool,
}
```

If a typed presentation enum is materially smaller after implementation context
is considered, use it instead. Do not introduce Ratatui types into the model.

**Step 3: Project each row type explicitly**

- Catalog: label, metadata from `entry_detail`, comment from `entry.comment`, kind.
- Group: label and formatted count metadata.
- Profile: label, profile kind, session-only provenance, status, endpoint.
- Status/load-more/empty: label and optional metadata only.

Delete `entry_display_detail` once no caller needs concatenated metadata/comment.
Delete `explorer_status_label` if status formatting moves entirely to the UI.

**Step 4: Run model-facing tests**

Run: `cargo test --test ui_render --test explorer_state --test explorer_performance`

Expected: compilation succeeds and projection assertions pass; render assertions
may still use the old visual hierarchy until Task 6.

### Task 6: Render Root Status, Nerd Font Icons, and Muted Comments as Spans

**Files:**
- Modify: `src/ui/mod.rs:462-569,1450-1477`
- Modify: `tests/ui_render.rs`

**Step 1: Add render-level assertions**

Add or update tests to verify visible text:

- saved roots do not contain `SAVED`
- session roots contain `SESSION`
- online roots show `●` without `ONLINE`
- offline roots show `○`
- linking shows `◐ CONNECTING`
- syncing shows `◐ SYNCING`
- failed shows `● FAILED`
- PostgreSQL, MySQL, and SQLite each render the chosen Nerd Font glyph
- exact count renders `79`, not `Exact(79)`
- catalog names do not append `database`, `schema`, or `table`

Use Ratatui buffer cell styles to assert a table comment uses `theme.muted` while
the table name uses its primary style. Plain rendered text cannot verify color.

**Step 2: Introduce centralized helpers**

Add focused UI helpers:

```rust
fn database_icon(kind: DatabaseKind) -> &'static str { ... }

fn connection_status_spans(
    status: ExplorerConnectionStatus,
    theme: Theme,
) -> Vec<Span<'static>> { ... }
```

Use the approved Nerd Font glyphs in `database_icon`. Status output must follow
the approved shape/text/color table.

**Step 3: Build rows from semantic spans**

In `render_explorer`:

1. Create indentation and expand marker spans.
2. Choose profile database icon or catalog icon.
3. Add the primary label span.
4. Add session provenance only when present.
5. Add connection status spans for profile roots.
6. Add endpoint, metadata, and comment spans in priority order.
7. Sanitize every dynamic string before span construction.
8. Apply selection background to every span while retaining foreground hierarchy.

Do not reconstruct a single combined text string. Bound secondary fields to the
remaining cell width and truncate comment/endpoint before the primary label.

**Step 4: Run render tests**

Run: `cargo test --test ui_render`

Expected: PASS at supported, compact, and hostile-metadata viewport sizes.

### Task 7: Update Contextual Help and Documentation

**Files:**
- Modify: `src/ui/mod.rs:993-1030,1316-1364`
- Modify: `tests/ui_render.rs:610-623`
- Modify: `docs/keybindings.md:39-73`
- Modify: `docs/configuration.md:21-27` if root labels are described there

**Step 1: Update failing help assertions**

Assert Explorer help contains distinct descriptions for:

```text
h / l          collapse / expand
o              toggle expand / collapse
Enter          open table preview / activate
```

Assert the footer includes `o toggle` and `Enter open`.

**Step 2: Implement help copy**

Split the existing combined `h / l / Enter` help row. Keep the help popup within
its current height; remove redundant copy rather than increasing modal size if
necessary.

**Step 3: Update Markdown documentation**

Document the new keyboard split, saved/session root behavior, status
presentation, relation descendant activation, count formatting, and comment
styling where appropriate.

**Step 4: Run UI/keymap documentation-adjacent tests**

Run: `cargo test --test ui_render --test keymap`

Expected: PASS.

### Task 8: Full Verification

**Files:**
- No new files

**Step 1: Format**

Run: `cargo fmt --check`

Expected: PASS.

**Step 2: Run focused Explorer and relation suites**

Run: `cargo test --test keymap --test ui_render --test explorer_state --test explorer_performance --test catalog_reducer --test relation_tabs --test relation_runtime`

Expected: PASS.

**Step 3: Run the complete suite**

Run: `cargo test`

Expected: PASS.

**Step 4: Run Clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS.

**Step 5: Inspect the final diff**

Run: `git diff --check`

Expected: PASS. Confirm unrelated untracked plan files remain untouched.

No commit is included unless explicitly requested by the user.
