# Keyboard Navigation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Align pane focus, SQL console tab navigation, and relation/result view shortcuts with the approved Vim-oriented design.

**Architecture:** Keep the existing `Action`-based input pipeline. Add standard `gt/gT` and page-key aliases in the global keymap, make `Ctrl-w h/j/k/l` directional in both global and modal-editor dispatch, and add scoped numeric result-view actions using the existing view-setting actions. Update help and keybinding documentation without introducing configurable keymaps.

**Tech Stack:** Rust, crossterm key events, existing LazyDB `Action` reducer, Rust unit/integration tests.

---

### Task 1: Add failing keymap tests for console tabs and result views

**Files:**
- Modify: `src/input/keymap.rs:1274-end`

**Step 1: Write the failing tests**

Add tests covering:

- `g` followed by `t` maps to `Action::NextTab`.
- `g` followed by shifted `T` maps to `Action::PreviousTab`.
- `Ctrl-PageDown` and `Ctrl-PageUp` map to next/previous tab.
- Results-focused `1`, `2`, and `3` map to the appropriate direct result-view action where supported.
- Numeric keys do not intercept data-query input or editor insert input.

Use the existing test helpers and app fixtures in `src/input/keymap.rs`; preserve the existing sequence timeout and focus preemption behavior.

**Step 2: Run tests to verify they fail**

Run: `cargo test input::keymap --lib`

Expected: the new tests fail because the bindings/actions are not implemented.

**Step 3: Commit the failing tests**

```bash
git add src/input/keymap.rs
git commit -m "test: specify keyboard navigation shortcuts"
```

### Task 2: Implement global console-tab and result-view mappings

**Files:**
- Modify: `src/input/keymap.rs:300-606,625-680,1113-1184`
- Modify: `src/action.rs` only if a direct result-view action is required by the existing reducer
- Modify: `src/app.rs` only if direct view actions need reducer handling

**Step 1: Add standard console-tab mappings**

In the global keymap, add `g` as a pending sequence and resolve:

- `gt` to `Action::NextTab`.
- `gT` to `Action::PreviousTab`.

Add modifier-specific mappings for `Ctrl-PageDown` and `Ctrl-PageUp`. Keep `[t` and `]t` unchanged as compatibility aliases.

Ensure mappings are evaluated only after overlays, completions, data-query inputs, and editor insert-mode handling have had priority.

**Step 2: Add scoped numeric result-view mappings**

When Results is focused and the active tab supports the requested view, map:

- `1` to Data.
- `2` to Output for SQL consoles or DDL for relation tabs.
- `3` to Plan when the existing result-view model supports it.

Prefer existing `Action::SetRelationView` and `Action::ToggleResultView` patterns. Add the smallest direct action needed if no current action can select SQL `ResultView::Data`, `Output`, or `Plan` directly. Unsupported selections must return `None`.

**Step 3: Run the focused tests**

Run: `cargo test input::keymap --lib`

Expected: PASS.

**Step 4: Commit the implementation**

```bash
git add src/input/keymap.rs src/action.rs src/app.rs
git commit -m "feat: improve tab and result navigation"
```

### Task 3: Normalize directional pane focus

**Files:**
- Modify: `src/input/keymap.rs:625-652`
- Modify: `src/editor/mod.rs:1127-1138`
- Modify: `src/help.rs:150-180`

**Step 1: Write or update directional focus tests**

Cover the layout-specific behavior:

- `Ctrl-w h` targets Explorer from the right-side panes.
- `Ctrl-w j` targets Results from Editor.
- `Ctrl-w k` targets Editor from Results.
- `Ctrl-w l` targets Editor from Explorer.
- Directional keys do not wrap at an edge.
- `Tab` and `Shift-Tab` retain cyclic behavior.

**Step 2: Implement the consistent mappings**

Update global and modal editor handling to use geometric direction rather than making `k` and `l` interchangeable absolute aliases. Avoid changing Insert mode `Ctrl-w`, which remains delete-previous-word.

**Step 3: Update help labels**

Describe `Ctrl-w h/j/k/l` as left/down/up/right or explicitly document the actual layout mapping. Keep `Tab` and `Shift-Tab` visible as cyclic focus navigation.

**Step 4: Run focused tests and format**

Run: `cargo test app::tests --lib`

Run: `cargo fmt --all -- --check`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/input/keymap.rs src/editor/mod.rs src/help.rs
git commit -m "fix: make pane focus directional"
```

### Task 4: Update keybinding documentation

**Files:**
- Modify: `docs/keybindings.md`

**Step 1: Document the final bindings**

Add a concise table for:

- Directional pane focus and cyclic focus.
- `gt/gT`, page-key aliases, and retained `[t`/`]t` aliases.
- Numeric result views, `o`, and relation `p`/`D` aliases.

Document scope and input-mode exceptions so users know why the same physical key can behave differently in the editor or query filters.

**Step 2: Review for stale bindings**

Search the document for `[t`, `]t`, `Ctrl-w`, `Data`, `DDL`, `Output`, and `Plan`; ensure each displayed primary binding matches the implementation.

**Step 3: Commit**

```bash
git add docs/keybindings.md
git commit -m "docs: update keyboard navigation reference"
```

### Task 5: Run complete verification

**Files:**
- No source changes expected unless verification exposes an issue.

**Step 1: Run formatting**

Run: `cargo fmt --all -- --check`

**Step 2: Run the full test suite**

Run: `cargo test --all-targets`

**Step 3: Run Clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: all commands pass. If repository CI documents a different standard command, use that command as well and record the result.

**Step 4: Inspect the final worktree**

Run: `git status --short --branch`

Run: `git log --oneline -5`

Confirm only the design, implementation plan, source, tests, and keybinding documentation commits exist on `task/keyboard-navigation`.
